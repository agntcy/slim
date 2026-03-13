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
    Channel, Context, DecodedStream, Decoder, Encoder, MulticastItem, RpcError, Server,
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
    /// Channel used as the multicast broadcaster.
    ///
    /// Created with `Channel::new_with_members_internal` so the GROUP session
    /// name is randomly generated and members are auto-invited on the first
    /// multicast call.
    channel: Channel,
}

impl MulticastTestEnv {
    async fn new(test_name: &str, num_members: usize) -> Self {
        let id = ID::new_with_name(Kind::new("slim").unwrap(), test_name).unwrap();
        let service = Arc::new(Service::new(id));

        // Create N member apps, each with a UNIQUE name ("org/ns/member-{i}").
        // Each app auto-subscribes to its own unique name via process_messages,
        // making it reachable for the invite discovery-request sent by the Channel.
        let mut member_servers = Vec::new();
        let mut member_app_names = Vec::new();
        for i in 0..num_members {
            let member_app_name = Name::from_strings(["org", "ns", &format!("member-{}", i)]);
            let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
            let (app, notifications) = service
                .create_app(
                    &member_app_name,
                    AuthProvider::shared_secret(secret.clone()),
                    AuthVerifier::shared_secret(secret),
                )
                .unwrap();
            let app = Arc::new(app);
            let server = Arc::new(Server::new_internal(
                app.clone(),
                member_app_name.clone(),
                notifications,
            ));
            member_app_names.push(member_app_name);
            member_servers.push(server);
        }

        // Broadcaster app — uses new_with_members_internal so the Channel
        // generates a random UUID group name and auto-invites all members on
        // the first multicast call.
        let client_name = Name::from_strings(["org", "ns", "client"]);
        let secret = SharedSecret::new("client", TEST_VALID_SECRET).unwrap();
        let (client_app, _) = service
            .create_app(
                &client_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret),
            )
            .unwrap();
        let channel =
            Channel::new_with_members_internal(Arc::new(client_app), member_app_names, true, None)
                .expect("failed to create channel");

        Self {
            service,
            member_servers,
            channel,
        }
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
        // Give all servers time to subscribe before the first invite is sent.
        tokio::time::sleep(Duration::from_millis(100)).await;
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

/// Collect exactly `n` items (successes or errors) from a multicast stream.
///
/// Unlike `collect_n_multicast`, this does NOT panic on errors — it stores them
/// so tests can assert on the mix of successes and failures.
async fn collect_n_mixed<T>(
    stream: impl futures::Stream<Item = Result<MulticastItem<T>, RpcError>>,
    n: usize,
    timeout: Duration,
    label: &str,
) -> Vec<Result<MulticastItem<T>, RpcError>> {
    pin_mut!(stream);
    let mut results: Vec<Result<MulticastItem<T>, RpcError>> = Vec::with_capacity(n);
    tokio::time::timeout(timeout, async {
        for _ in 0..n {
            match stream.next().await {
                Some(item) => results.push(item),
                None => break,
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "{label}: timed out after collecting {}/{n} items",
            results.len()
        )
    });
    results
}

/// Collect exactly `n` responses from a multicast stream, failing if:
/// - the stream ends before `n` items arrive, or
/// - any item is an error, or
/// - the operation does not complete within `timeout`.
async fn collect_n_multicast<T>(
    stream: impl futures::Stream<Item = Result<MulticastItem<T>, RpcError>>,
    n: usize,
    timeout: Duration,
    label: &str,
) -> Vec<MulticastItem<T>> {
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
    .unwrap_or_else(|_| {
        panic!(
            "{label}: timed out after collecting {}/{n} items",
            responses.len()
        )
    });
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

    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "hello".to_string(),
            value: 10,
        },
        Some(Duration::from_secs(10)),
        None,
    );

    let mut responses = collect_n_multicast(
        stream,
        NUM_MEMBERS,
        Duration::from_secs(10),
        "multicast_unary",
    )
    .await;
    responses.sort_by_key(|r| r.message.member_id);

    assert_eq!(responses.len(), NUM_MEMBERS);
    assert_eq!(responses[0].message.result, "M0: hello");
    assert_eq!(responses[0].message.count, 10);
    assert_eq!(responses[1].message.result, "M1: hello");
    assert_eq!(responses[1].message.count, 11);
    // Source should identify each member by name.
    assert_eq!(
        responses[0].context.source,
        Name::from_strings(["org", "ns", "member-0"])
    );
    assert_eq!(
        responses[1].context.source,
        Name::from_strings(["org", "ns", "member-1"])
    );

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

    let stream = env
        .channel
        .multicast_unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "Expand",
            TestRequest {
                message: "x".to_string(),
                value: 1,
            },
            Some(Duration::from_secs(10)),
            None,
        );

    let total = NUM_MEMBERS * ITEMS_PER_MEMBER;
    let responses = collect_n_multicast(
        stream,
        total,
        Duration::from_secs(10),
        "multicast_unary_stream",
    )
    .await;

    assert_eq!(responses.len(), total);
    // Each member contributed exactly ITEMS_PER_MEMBER items.
    for mid in 0..NUM_MEMBERS {
        let count = responses
            .iter()
            .filter(|r| r.message.member_id == mid)
            .count();
        assert_eq!(
            count, ITEMS_PER_MEMBER,
            "member {mid} should have sent {ITEMS_PER_MEMBER} items"
        );
        // Every item from this member should carry the expected source name.
        let expected_src = Name::from_strings(["org", "ns", &format!("member-{mid}")]);
        assert!(
            responses
                .iter()
                .filter(|r| r.message.member_id == mid)
                .all(|r| r.context.source == expected_src),
            "member {mid} items have wrong source"
        );
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
            move |mut req_stream: DecodedStream<TestRequest>, _ctx: Context| async move {
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

    let requests = stream::iter(vec![
        TestRequest {
            message: "a".to_string(),
            value: 1,
        },
        TestRequest {
            message: "b".to_string(),
            value: 2,
        },
        TestRequest {
            message: "c".to_string(),
            value: 3,
        },
    ]);

    let stream = env
        .channel
        .multicast_stream_unary::<TestRequest, TestResponse>(
            "TestService",
            "Sum",
            requests,
            Some(Duration::from_secs(10)),
            None,
        );

    let mut responses = collect_n_multicast(
        stream,
        NUM_MEMBERS,
        Duration::from_secs(10),
        "multicast_stream_unary",
    )
    .await;
    responses.sort_by_key(|r| r.message.member_id);

    assert_eq!(responses.len(), NUM_MEMBERS);
    for r in &responses {
        assert_eq!(
            r.message.count, 6,
            "member {} should sum to 6",
            r.message.member_id
        );
        assert!(
            r.message.result.contains("a+b+c"),
            "member {} got: {}",
            r.message.member_id,
            r.message.result
        );
        let expected_src =
            Name::from_strings(["org", "ns", &format!("member-{}", r.message.member_id)]);
        assert_eq!(r.context.source, expected_src, "wrong source for member");
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
            move |mut req_stream: DecodedStream<TestRequest>, _ctx: Context| async move {
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

    let requests = stream::iter(vec![
        TestRequest {
            message: "x".to_string(),
            value: 1,
        },
        TestRequest {
            message: "y".to_string(),
            value: 2,
        },
        TestRequest {
            message: "z".to_string(),
            value: 3,
        },
    ]);

    let stream = env
        .channel
        .multicast_stream_stream::<TestRequest, TestResponse>(
            "TestService",
            "Echo",
            requests,
            Some(Duration::from_secs(10)),
            None,
        );

    let total = NUM_MEMBERS * NUM_REQUESTS;
    let responses = collect_n_multicast(
        stream,
        total,
        Duration::from_secs(10),
        "multicast_stream_stream",
    )
    .await;

    assert_eq!(responses.len(), total);
    for mid in 0..NUM_MEMBERS {
        let member_responses: Vec<_> = responses
            .iter()
            .filter(|r| r.message.member_id == mid)
            .collect();
        assert_eq!(member_responses.len(), NUM_REQUESTS);
        let values: Vec<i32> = member_responses.iter().map(|r| r.message.count).collect();
        // Each member should have echoed all 3 request values.
        for v in [1, 2, 3] {
            assert!(values.contains(&v), "member {mid} missing value {v}");
        }
        let expected_src = Name::from_strings(["org", "ns", &format!("member-{mid}")]);
        assert!(
            member_responses
                .iter()
                .all(|r| r.context.source == expected_src),
            "member {mid} items have wrong source"
        );
    }

    env.shutdown().await;
}

// ============================================================================
// Test 5 — partial error, unary pattern
// ============================================================================

/// One member returns an error; the other members' successes must still arrive.
///
/// Uses `multicast_unary` (unary-unary). The server sends one data message per
/// member — no EOS marker. The caller collects exactly NUM_MEMBERS items
/// (mix of Ok and Err) and then drops the stream.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_partial_error_unary() {
    const NUM_MEMBERS: usize = 3;
    let mut env = MulticastTestEnv::new("test-multicast-partial-error-unary", NUM_MEMBERS).await;

    // Member 0 returns an error.
    env.member_servers[0].register_unary_unary_internal(
        "TestService",
        "Echo",
        move |_req: TestRequest, _ctx: Context| async move {
            Err::<TestResponse, _>(RpcError::internal("member 0 failed"))
        },
    );
    // Members 1 and 2 succeed.
    for i in 1..NUM_MEMBERS {
        env.member_servers[i].register_unary_unary_internal(
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

    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "hello".to_string(),
            value: 10,
        },
        Some(Duration::from_secs(10)),
        None,
    );

    let results = collect_n_mixed(
        stream,
        NUM_MEMBERS,
        Duration::from_secs(10),
        "partial_error_unary",
    )
    .await;

    assert_eq!(results.len(), NUM_MEMBERS);
    let errors: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(errors.len(), 1, "expected exactly 1 error");
    assert_eq!(successes.len(), 2, "expected 2 successes");

    env.shutdown().await;
}

// ============================================================================
// Test 6 — partial error, streaming pattern
// ============================================================================

/// One member errors mid-stream; the other member's full response stream must
/// still arrive and the combined stream must terminate cleanly.
///
/// Uses `multicast_unary_stream`. Member 0 yields two items then an error;
/// member 1 yields three items then EOS. The client should receive:
///   - 2 Ok items from member 0
///   - 1 Err from member 0 (counted as its EOS)
///   - 3 Ok items from member 1
///   - stream terminates after member 1's EOS
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_partial_error_unary_stream() {
    const NUM_MEMBERS: usize = 2;
    const M0_OK_ITEMS: usize = 2;
    const M1_OK_ITEMS: usize = 3;
    let mut env =
        MulticastTestEnv::new("test-multicast-partial-error-unary-stream", NUM_MEMBERS).await;

    // Member 0: 2 ok items then an error (no server-side EOS follows the error).
    env.member_servers[0].register_unary_stream_internal(
        "TestService",
        "Expand",
        move |req: TestRequest, _ctx: Context| async move {
            let items: Vec<Result<TestResponse, RpcError>> = (0..M0_OK_ITEMS)
                .map(|j| {
                    Ok(TestResponse {
                        member_id: 0,
                        result: format!("M0-item{j}: {}", req.message),
                        count: j as i32,
                    })
                })
                .chain(std::iter::once(Err(RpcError::internal("M0 stream error"))))
                .collect();
            Ok(stream::iter(items))
        },
    );
    // Member 1: 3 ok items, server sends EOS after the last one.
    env.member_servers[1].register_unary_stream_internal(
        "TestService",
        "Expand",
        move |req: TestRequest, _ctx: Context| async move {
            let items: Vec<Result<TestResponse, RpcError>> = (0..M1_OK_ITEMS)
                .map(|j| {
                    Ok(TestResponse {
                        member_id: 1,
                        result: format!("M1-item{j}: {}", req.message),
                        count: j as i32,
                    })
                })
                .collect();
            Ok(stream::iter(items))
        },
    );
    env.start_all_servers().await;

    let stream = env
        .channel
        .multicast_unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "Expand",
            TestRequest {
                message: "x".to_string(),
                value: 1,
            },
            Some(Duration::from_secs(10)),
            None,
        );

    // M0 yields M0_OK_ITEMS Ok + 1 Err; M1 yields M1_OK_ITEMS Ok.
    // The stream terminates after M1's EOS (M0's error is counted as its EOS),
    // so the total item count is known up front.
    let total = M0_OK_ITEMS + 1 + M1_OK_ITEMS;
    let results = collect_n_mixed(
        stream,
        total,
        Duration::from_secs(10),
        "partial_error_stream",
    )
    .await;

    let errors: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(errors.len(), 1, "expected exactly 1 error (from M0)");
    assert_eq!(
        successes.len(),
        M0_OK_ITEMS + M1_OK_ITEMS,
        "expected all ok items from both members"
    );
    let m1_items: Vec<_> = successes
        .iter()
        .filter(|r| r.as_ref().unwrap().message.member_id == 1)
        .collect();
    assert_eq!(m1_items.len(), M1_OK_ITEMS, "M1 should deliver all items");

    env.shutdown().await;
}
