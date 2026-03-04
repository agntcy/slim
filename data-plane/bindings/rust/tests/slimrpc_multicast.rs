// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC multicast / GROUP RPC patterns
//!
//! Tests the four multicast interaction patterns plus the group-inbox observer:
//! - multicast_unary:        one request broadcast to all members, one response per member
//! - multicast_unary_stream: one request, each member streams multiple responses
//! - multicast_stream_unary: client streams requests, one response per member
//! - multicast_stream_stream: client streams requests, each member streams responses
//! - group_inbox:            a member can observe other members' responses via subscribe_group_inbox()
//!
//! Topology
//! --------
//! All tests share the same shape:
//!   - A shared in-process SLIM `Service` acts as the message bus.
//!   - Multiple "member" apps are registered under the SAME name ("org/ns/member").
//!     Each has a `Server` that handles incoming requests.
//!   - A separate "client" app holds a `Channel` that broadcasts multicast RPCs.
//!   - (group_inbox test only) An "observer" app also opens a multicast Channel
//!     to the same group name and subscribes to the group inbox.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::pin_mut;
use futures::stream;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::id::{ID, Kind};
use slim_datapath::messages::Name;
use slim_service::service::Service;
use slim_testing::utils::TEST_VALID_SECRET;

use slim_bindings::slimrpc::{
    Channel, Context, Decoder, Encoder, RequestStream, RpcError, Server,
};

// ============================================================================
// Test message types
// ============================================================================

#[derive(Debug, Clone, Default, PartialEq, bincode::Encode, bincode::Decode)]
struct TestRequest {
    pub message: String,
    pub value: i32,
}

impl Encoder for TestRequest {
    fn encode(self) -> Result<Vec<u8>, RpcError> {
        bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| RpcError::internal(format!("Encoding error: {}", e)))
    }
}

impl Decoder for TestRequest {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> {
        let (v, _): (TestRequest, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| RpcError::invalid_argument(format!("Decoding error: {}", e)))?;
        Ok(v)
    }
}

#[derive(Debug, Clone, Default, PartialEq, bincode::Encode, bincode::Decode)]
struct TestResponse {
    pub member_id: usize,
    pub result: String,
    pub count: i32,
}

impl Encoder for TestResponse {
    fn encode(self) -> Result<Vec<u8>, RpcError> {
        bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| RpcError::internal(format!("Encoding error: {}", e)))
    }
}

impl Decoder for TestResponse {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> {
        let (v, _): (TestResponse, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| RpcError::invalid_argument(format!("Decoding error: {}", e)))?;
        Ok(v)
    }
}

// ============================================================================
// MulticastTestEnv
// ============================================================================

/// Test environment with `num_members` servers and one broadcaster channel.
struct MulticastTestEnv {
    service: Arc<Service>,
    /// Member servers — each registered under its own unique app name.
    member_servers: Vec<Arc<Server>>,
    /// Individual app names for each member (used for per-member invites).
    member_app_names: Vec<Name>,
    /// Channel used as the multicast broadcaster (session targets `group_name`).
    channel: Channel,
    /// The GROUP session channel name (the `remote` the Channel targets).
    group_name: Name,
}

impl MulticastTestEnv {
    async fn new(test_name: &str, num_members: usize) -> Self {
        let id = ID::new_with_name(Kind::new("slim").unwrap(), test_name).unwrap();
        let service = Arc::new(Service::new(id));

        // The group/topic name used as the multicast session destination and
        // as the base_name for Server subscriptions.
        let group_name = Name::from_strings(["org", "ns", "member"]);
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();

        // Create N member apps, each with a UNIQUE name ("org/ns/member-{i}").
        // The unique name allows the broadcaster to invite each member
        // individually via invite_participant.  The Server still subscribes to
        // `group_name` (base_name) so RPC method routing works correctly, and
        // the app auto-subscribes to its own unique name (via process_messages)
        // so it is reachable for the invite discovery-request.
        let mut member_servers = Vec::new();
        let mut member_app_names = Vec::new();
        for i in 0..num_members {
            let member_app_name =
                Name::from_strings(["org", "ns", &format!("member-{}", i)]);
            let (app, notifications) = service
                .create_app(
                    &member_app_name,
                    AuthProvider::shared_secret(secret.clone()),
                    AuthVerifier::shared_secret(secret.clone()),
                )
                .unwrap();
            let app = Arc::new(app);
            // Server subscribes to group_name for RPC method routing.
            let server = Arc::new(Server::new_internal(
                app.clone(),
                group_name.clone(),
                notifications,
            ));
            member_app_names.push(member_app_name);
            member_servers.push(server);
        }

        // Broadcaster app — different name, Channel targeting the group name.
        let client_name = Name::from_strings(["org", "ns", "client"]);
        let (client_app, _) = service
            .create_app(
                &client_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret.clone()),
            )
            .unwrap();
        let channel = Channel::new_internal(Arc::new(client_app), group_name.clone());

        Self { service, member_servers, member_app_names, channel, group_name }
    }

    /// Start all member servers in background tasks.
    async fn start_all_servers(&self) {
        for server in &self.member_servers {
            let s = server.clone();
            tokio::spawn(async move {
                if let Err(e) = s.serve_async().await {
                    tracing::error!("Member server error: {:?}", e);
                }
            });
        }
        // Give all servers time to subscribe.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    /// Invite all members into the GROUP session.
    ///
    /// Sends one invite per member (by their unique app name). Each invite is
    /// a unicast discovery-request so every member joins the session.
    ///
    /// Call this after `start_all_servers()` and before making multicast RPC
    /// calls. A 200 ms sleep is included after all invites so every member has
    /// time to complete the join handshake.
    async fn invite_members(&self) {
        for name in &self.member_app_names {
            self.channel
                .invite_participant(name.clone())
                .await
                .expect("invite_participant failed");
        }
        // Let all members finish joining the session.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    /// Create an additional Channel for an observer app (passive-join mode).
    ///
    /// The returned channel will wait for a `NewSession` invite when
    /// `subscribe_group_inbox()` is first called. The caller must invite it
    /// explicitly via `self.channel.invite_participant(observer_name)` before
    /// that call (or concurrently with it).
    ///
    /// Returns `(channel, observer_name)` so the caller can issue the invite.
    async fn new_observer_channel(&self) -> (Channel, Name) {
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
        let observer_name = Name::from_strings(["org", "ns", "observer"]);
        let (observer_app, notifications) = self
            .service
            .create_app(
                &observer_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret.clone()),
            )
            .unwrap();
        let channel = Channel::new_internal_with_notifications(
            Arc::new(observer_app),
            self.group_name.clone(),
            notifications,
        );
        (channel, observer_name)
    }

    async fn shutdown(&mut self) {
        for server in &self.member_servers {
            server.shutdown_internal().await;
        }
        self.service.shutdown().await.unwrap();
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Collect exactly `n` responses from a multicast stream, failing if:
/// - the stream ends before `n` items arrive, or
/// - any item is an error, or
/// - the operation does not complete within `timeout`.
async fn collect_n_multicast<T>(
    stream: impl futures::Stream<Item = Result<T, RpcError>>,
    n: usize,
    timeout: Duration,
    label: &str,
) -> Vec<T> {
    pin_mut!(stream);
    let mut responses = Vec::with_capacity(n);
    tokio::time::timeout(timeout, async {
        for i in 0..n {
            match stream.next().await {
                Some(Ok(r)) => responses.push(r),
                Some(Err(e)) => panic!("{label}: item {i} failed: {e:?}"),
                None => panic!("{label}: stream ended after {i} items, expected {n}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label}: timed out after collecting {}/{n} items", responses.len()));
    responses
}

// ============================================================================
// Test 1 — multicast_unary
// ============================================================================

/// Broadcast one request; each member returns one response.
/// The client collects one `TestResponse` per member.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_unary() {
    const NUM_MEMBERS: usize = 2;
    let mut env = MulticastTestEnv::new("test-multicast-unary", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_unary_unary_internal(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context| async move {
                Ok(TestResponse {
                    member_id: i,
                    result: format!("M{i}: {}", req.message),
                    count: req.value + i as i32,
                })
            },
        );
    }
    env.start_all_servers().await;
    env.invite_members().await;

    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest { message: "hello".to_string(), value: 10 },
        Some(Duration::from_secs(10)),
        None,
    );

    let mut responses = collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "multicast_unary").await;
    responses.sort_by_key(|r| r.member_id);

    assert_eq!(responses.len(), NUM_MEMBERS);
    assert_eq!(responses[0].result, "M0: hello");
    assert_eq!(responses[0].count, 10);
    assert_eq!(responses[1].result, "M1: hello");
    assert_eq!(responses[1].count, 11);

    env.shutdown().await;
}

// ============================================================================
// Test 2 — multicast_unary_stream
// ============================================================================

/// Broadcast one request; each member returns a stream of responses.
/// The client interleaves all per-member streams into one stream.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_unary_stream() {
    const NUM_MEMBERS: usize = 2;
    const ITEMS_PER_MEMBER: usize = 3;
    let mut env = MulticastTestEnv::new("test-multicast-unary-stream", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_unary_stream_internal(
            "TestService",
            "Expand",
            move |req: TestRequest, _ctx: Context| async move {
                let items: Vec<Result<TestResponse, RpcError>> = (0..ITEMS_PER_MEMBER)
                    .map(|j| {
                        Ok(TestResponse {
                            member_id: i,
                            result: format!("M{i}-item{j}: {}", req.message),
                            count: req.value * 10 + j as i32,
                        })
                    })
                    .collect();
                Ok(stream::iter(items))
            },
        );
    }
    env.start_all_servers().await;
    env.invite_members().await;

    let stream = env.channel.multicast_unary_stream::<TestRequest, TestResponse>(
        "TestService",
        "Expand",
        TestRequest { message: "x".to_string(), value: 1 },
        Some(Duration::from_secs(10)),
        None,
    );

    let total = NUM_MEMBERS * ITEMS_PER_MEMBER;
    let responses = collect_n_multicast(stream, total, Duration::from_secs(10), "multicast_unary_stream").await;

    assert_eq!(responses.len(), total);
    // Each member contributed exactly ITEMS_PER_MEMBER items.
    for mid in 0..NUM_MEMBERS {
        let count = responses.iter().filter(|r| r.member_id == mid).count();
        assert_eq!(count, ITEMS_PER_MEMBER, "member {mid} should have sent {ITEMS_PER_MEMBER} items");
    }

    env.shutdown().await;
}

// ============================================================================
// Test 3 — multicast_stream_unary
// ============================================================================

/// Client streams requests to all members; each member aggregates and replies once.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_stream_unary() {
    const NUM_MEMBERS: usize = 2;
    let mut env = MulticastTestEnv::new("test-multicast-stream-unary", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_stream_unary_internal(
            "TestService",
            "Sum",
            move |mut req_stream: RequestStream<TestRequest>, _ctx: Context| async move {
                let mut total = 0i32;
                let mut msgs = Vec::new();
                while let Some(item) = req_stream.next().await {
                    let req = item?;
                    total += req.value;
                    msgs.push(req.message.clone());
                }
                Ok(TestResponse {
                    member_id: i,
                    result: format!("M{i}: {}", msgs.join("+")),
                    count: total,
                })
            },
        );
    }
    env.start_all_servers().await;
    env.invite_members().await;

    let requests = stream::iter(vec![
        TestRequest { message: "a".to_string(), value: 1 },
        TestRequest { message: "b".to_string(), value: 2 },
        TestRequest { message: "c".to_string(), value: 3 },
    ]);

    let stream = env.channel.multicast_stream_unary::<TestRequest, TestResponse>(
        "TestService",
        "Sum",
        requests,
        Some(Duration::from_secs(10)),
        None,
    );

    let mut responses = collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "multicast_stream_unary").await;
    responses.sort_by_key(|r| r.member_id);

    assert_eq!(responses.len(), NUM_MEMBERS);
    for r in &responses {
        assert_eq!(r.count, 6, "member {} should sum to 6", r.member_id);
        assert!(r.result.contains("a+b+c"), "member {} got: {}", r.member_id, r.result);
    }

    env.shutdown().await;
}

// ============================================================================
// Test 4 — multicast_stream_stream
// ============================================================================

/// Client streams requests; each member streams one response per request item.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_stream_stream() {
    const NUM_MEMBERS: usize = 2;
    const NUM_REQUESTS: usize = 3;
    let mut env = MulticastTestEnv::new("test-multicast-stream-stream", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_stream_stream_internal(
            "TestService",
            "Echo",
            move |mut req_stream: RequestStream<TestRequest>, _ctx: Context| async move {
                let responses: Vec<Result<TestResponse, RpcError>> = {
                    let mut v = Vec::new();
                    while let Some(item) = req_stream.next().await {
                        let req = item?;
                        v.push(Ok(TestResponse {
                            member_id: i,
                            result: format!("M{i}: {}", req.message),
                            count: req.value,
                        }));
                    }
                    v
                };
                Ok(stream::iter(responses))
            },
        );
    }
    env.start_all_servers().await;
    env.invite_members().await;

    let requests = stream::iter(vec![
        TestRequest { message: "x".to_string(), value: 1 },
        TestRequest { message: "y".to_string(), value: 2 },
        TestRequest { message: "z".to_string(), value: 3 },
    ]);

    let stream = env.channel.multicast_stream_stream::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        requests,
        Some(Duration::from_secs(10)),
        None,
    );

    let total = NUM_MEMBERS * NUM_REQUESTS;
    let responses = collect_n_multicast(stream, total, Duration::from_secs(10), "multicast_stream_stream").await;

    assert_eq!(responses.len(), total);
    for mid in 0..NUM_MEMBERS {
        let member_responses: Vec<_> = responses.iter().filter(|r| r.member_id == mid).collect();
        assert_eq!(member_responses.len(), NUM_REQUESTS);
        let values: Vec<i32> = member_responses.iter().map(|r| r.count).collect();
        // Each member should have echoed all 3 request values.
        for v in [1, 2, 3] {
            assert!(values.contains(&v), "member {mid} missing value {v}");
        }
    }

    env.shutdown().await;
}

// ============================================================================
// Test 5 — group inbox
// ============================================================================

/// An observer app opens a multicast Channel to the same group name and
/// subscribes to the group inbox.  When the broadcaster makes a multicast RPC
/// call the member servers respond; those responses circulate through the GROUP
/// session.  Because the observer has no active RPC with the same rpc-id, the
/// responses are forwarded to its group inbox instead of being dropped.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_group_inbox_sees_other_members_responses() {
    const NUM_MEMBERS: usize = 2;
    let mut env = MulticastTestEnv::new("test-group-inbox", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_unary_unary_internal(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context| async move {
                Ok(TestResponse {
                    member_id: i,
                    result: format!("M{i}: {}", req.message),
                    count: req.value,
                })
            },
        );
    }
    env.start_all_servers().await;
    env.invite_members().await;

    // Create an observer Channel in passive-join mode.  The observer waits
    // for a NewSession invite rather than creating its own GROUP session.
    let (observer, observer_name) = env.new_observer_channel().await;

    // Invite the observer into the broadcaster's GROUP session.
    // The invite message arrives at the observer app's notification queue; it
    // will be picked up by subscribe_group_inbox() below.
    env.channel
        .invite_participant(observer_name)
        .await
        .expect("invite observer failed");

    // Give the observer's app time to process the invite and buffer the
    // NewSession notification before subscribe_group_inbox reads it.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // subscribe_group_inbox() reads the buffered NewSession notification and
    // sets up the unrouted-message sink on the observer's dispatcher.
    let inbox = observer
        .subscribe_group_inbox()
        .await
        .expect("subscribe_group_inbox failed");
    pin_mut!(inbox);

    // Broadcaster sends a multicast request and collects both responses.
    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest { message: "ping".to_string(), value: 7 },
        Some(Duration::from_secs(10)),
        None,
    );
    let broadcaster_responses = collect_n_multicast(
        stream,
        NUM_MEMBERS,
        Duration::from_secs(10),
        "group_inbox/broadcaster",
    )
    .await;
    assert_eq!(broadcaster_responses.len(), NUM_MEMBERS);

    // The observer's inbox should receive the same response messages: they
    // arrived on the GROUP session with rpc-ids unknown to the observer.
    let inbox_msgs = tokio::time::timeout(Duration::from_secs(5), async {
        let mut msgs = Vec::new();
        for _ in 0..NUM_MEMBERS {
            match inbox.next().await {
                Some(m) => msgs.push(m),
                None => break,
            }
        }
        msgs
    })
    .await
    .expect("Timed out waiting for group inbox messages");

    assert_eq!(
        inbox_msgs.len(),
        NUM_MEMBERS,
        "Observer inbox should have received one message per member"
    );

    // Verify the inbox messages carry non-empty payloads (the encoded responses).
    for msg in &inbox_msgs {
        assert!(!msg.payload.is_empty(), "Inbox message should have a payload");
    }

    env.shutdown().await;
}
