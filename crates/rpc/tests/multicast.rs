// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Exercises the native (non-uniffi) RPC API; the uniffi trait-object suite
// lives in handlers.rs.
#![cfg(not(feature = "uniffi"))]

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
use slim_datapath::api::ProtoName as Name;
use slim_service::service::Service;

const TEST_VALID_SECRET: &str = "test-shared-secret-value-0123456789abcdef";

use slim_rpc::{
    Channel, Context, DecodedStream, Decoder, Encoder, MulticastItem, PeerMessage,
    PeerResponseStream, RpcError, Server,
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
            .map_err(|e| RpcError::internal(format!("Encoding error: {e}")))
    }
}

impl Decoder for TestRequest {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> {
        let (v, _): (TestRequest, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| RpcError::invalid_argument(format!("Decoding error: {e}")))?;
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
            .map_err(|e| RpcError::internal(format!("Encoding error: {e}")))
    }
}

impl Decoder for TestResponse {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> {
        let (v, _): (TestResponse, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| RpcError::invalid_argument(format!("Decoding error: {e}")))?;
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
    /// Created with `Channel::new_with_members` so the GROUP session
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
            let member_app_name = Name::from_strings(["org", "ns", &format!("member-{i}")]);
            let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
            let (app, notifications) = service
                .create_app(
                    &member_app_name,
                    AuthProvider::shared_secret(secret.clone()),
                    AuthVerifier::shared_secret(secret),
                )
                .unwrap();
            let app = Arc::new(app);
            let server = Arc::new(Server::new(
                app.clone(),
                member_app_name.clone(),
                notifications,
            ));
            member_app_names.push(member_app_name);
            member_servers.push(server);
        }

        // Broadcaster app — uses new_with_members so the Channel
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
        let channel = Channel::new_with_members(Arc::new(client_app), member_app_names, true, None)
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
                if let Err(e) = s.serve().await {
                    tracing::error!("Member server error: {:?}", e);
                }
            });
        }
        // Give all servers time to subscribe before the first invite is sent.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    async fn shutdown(&mut self) {
        self.channel.close(None).await.unwrap();

        for server in &self.member_servers {
            server.shutdown().await;
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
        server.register_unary_unary(
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
        server.register_unary_stream(
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
        server.register_stream_unary(
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
        server.register_stream_stream(
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
    env.member_servers[0].register_unary_unary(
        "TestService",
        "Echo",
        move |_req: TestRequest, _ctx: Context| async move {
            Err::<TestResponse, _>(RpcError::internal("member 0 failed"))
        },
    );
    // Members 1 and 2 succeed.
    for i in 1..NUM_MEMBERS {
        env.member_servers[i].register_unary_unary(
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
    env.member_servers[0].register_unary_stream(
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
    env.member_servers[1].register_unary_stream(
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

// ============================================================================
// Test 7 — MulticastRpc error carries origin (unary)
// ============================================================================

/// When a member returns an error in a multicast unary call, the yielded
/// `RpcError` must be the `MulticastRpc` variant with the `origin` field set
/// to the failing member's name.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_error_origin_unary() {
    const NUM_MEMBERS: usize = 3;
    let mut env = MulticastTestEnv::new("test-multicast-error-origin-unary", NUM_MEMBERS).await;

    // Member 0 returns an error.
    env.member_servers[0].register_unary_unary(
        "TestService",
        "Echo",
        move |_req: TestRequest, _ctx: Context| async move {
            Err::<TestResponse, _>(RpcError::internal("member 0 exploded"))
        },
    );
    // Members 1 and 2 succeed.
    for i in 1..NUM_MEMBERS {
        env.member_servers[i].register_unary_unary(
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
        "error_origin_unary",
    )
    .await;

    assert_eq!(results.len(), NUM_MEMBERS);

    let errors: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(errors.len(), 1, "expected exactly 1 error");

    let err = &errors[0];
    let expected_origin = Name::from_strings(["org", "ns", "member-0"]).to_string();
    match err {
        RpcError::MulticastRpc {
            origin,
            code,
            message,
            ..
        } => {
            assert_eq!(
                origin, &expected_origin,
                "origin must identify the failing member"
            );
            assert_eq!(*code, slim_rpc::RpcCode::Internal);
            assert!(
                message.contains("member 0 exploded"),
                "message should be preserved"
            );
        }
        other => panic!("expected MulticastRpc, got: {other:?}"),
    }

    // Successes should still be plain Ok items with correct context.
    let successes: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    assert_eq!(successes.len(), 2, "expected 2 successes");

    env.shutdown().await;
}

// ============================================================================
// Test 8 — MulticastRpc error carries origin (streaming)
// ============================================================================

/// When a member errors mid-stream, the yielded error must be `MulticastRpc`
/// with the correct `origin`.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_error_origin_stream() {
    const NUM_MEMBERS: usize = 2;
    const M0_OK_ITEMS: usize = 1;
    const M1_OK_ITEMS: usize = 2;
    let mut env = MulticastTestEnv::new("test-multicast-error-origin-stream", NUM_MEMBERS).await;

    // Member 0: 1 ok item then an error.
    env.member_servers[0].register_unary_stream(
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
                .chain(std::iter::once(Err(RpcError::internal("M0 broke"))))
                .collect();
            Ok(stream::iter(items))
        },
    );
    // Member 1: 2 ok items, normal completion.
    env.member_servers[1].register_unary_stream(
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

    let total = M0_OK_ITEMS + 1 + M1_OK_ITEMS;
    let results = collect_n_mixed(
        stream,
        total,
        Duration::from_secs(10),
        "error_origin_stream",
    )
    .await;

    let errors: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(errors.len(), 1, "expected exactly 1 error (from M0)");

    let err = &errors[0];
    let expected_origin = Name::from_strings(["org", "ns", "member-0"]).to_string();
    match err {
        RpcError::MulticastRpc { origin, code, .. } => {
            assert_eq!(origin, &expected_origin, "origin must identify M0");
            assert_eq!(*code, slim_rpc::RpcCode::Internal);
        }
        other => panic!("expected MulticastRpc, got: {other:?}"),
    }

    env.shutdown().await;
}

// ============================================================================
// Test 9 — MulticastRpc error carries origin for all failing members
// ============================================================================

/// When multiple members fail, each error must carry its own origin.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_error_origin_multiple_failures() {
    const NUM_MEMBERS: usize = 3;
    let mut env = MulticastTestEnv::new("test-multicast-error-origin-multi", NUM_MEMBERS).await;

    // All members return errors with distinct messages.
    for i in 0..NUM_MEMBERS {
        env.member_servers[i].register_unary_unary(
            "TestService",
            "Echo",
            move |_req: TestRequest, _ctx: Context| async move {
                Err::<TestResponse, _>(RpcError::internal(format!("fail-{i}")))
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
        "error_origin_multi",
    )
    .await;

    assert_eq!(results.len(), NUM_MEMBERS);

    // Every result should be a MulticastRpc error.
    let mut origins: Vec<String> = Vec::new();
    for result in &results {
        match result {
            Err(RpcError::MulticastRpc { origin, .. }) => {
                origins.push(origin.clone());
            }
            Err(other) => panic!("expected MulticastRpc, got: {other:?}"),
            Ok(item) => panic!("expected error, got success: {:?}", item.message),
        }
    }

    // Each member should appear exactly once.
    origins.sort();
    let mut expected: Vec<String> = (0..NUM_MEMBERS)
        .map(|i| Name::from_strings(["org", "ns", &format!("member-{i}")]).to_string())
        .collect();
    expected.sort();
    assert_eq!(
        origins, expected,
        "each failing member must appear as an origin"
    );

    env.shutdown().await;
}

// ============================================================================
// Test 10 — channel close: idle (no session)
// ============================================================================

/// `close` on a channel that has never been used must succeed immediately
/// without panicking or blocking.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_channel_close_no_session() {
    let mut env = MulticastTestEnv::new("test-channel-close-no-session", 1).await;

    env.member_servers[0].register_unary_unary(
        "TestService",
        "Echo",
        move |req: TestRequest, _ctx: Context| async move {
            Ok(TestResponse {
                member_id: 0,
                result: req.message,
                count: req.value,
            })
        },
    );
    env.start_all_servers().await;

    // No RPC has been made — no underlying session exists yet.
    env.channel
        .close(None)
        .await
        .expect("close on idle channel must succeed");

    env.shutdown().await;
}

// ============================================================================
// Test 11 — channel close: active session, then reuse
// ============================================================================

/// After `close` on a channel with an active session the channel must
/// still be usable: the next RPC call re-creates the session transparently.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_channel_close_after_rpc() {
    const NUM_MEMBERS: usize = 2;
    let mut env = MulticastTestEnv::new("test-channel-close-after-rpc", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_unary_unary(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context| async move {
                Ok(TestResponse {
                    member_id: i,
                    result: req.message,
                    count: req.value,
                })
            },
        );
    }
    env.start_all_servers().await;

    // First call — establishes the persistent GROUP session.
    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "first".to_string(),
            value: 1,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    let responses =
        collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "first call").await;
    assert_eq!(responses.len(), NUM_MEMBERS);

    // Close the session — the dispatcher task must exit naturally.
    env.channel
        .close(None)
        .await
        .expect("close must succeed with an active session");

    // Second call — channel must re-create the session transparently.
    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "second".to_string(),
            value: 2,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    let responses =
        collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "second call").await;
    assert_eq!(responses.len(), NUM_MEMBERS);
    assert!(responses.iter().all(|r| r.message.result == "second"));

    env.shutdown().await;
}

// ============================================================================
// Shared-responses helpers
// ============================================================================

/// Build a fresh `MulticastTestEnv` whose channel uses shared-responses mode
/// and whose servers opt in via `Server::new_with_shared_responses`.
async fn new_shared_env(test_name: &str, num_members: usize) -> MulticastTestEnv {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), test_name).unwrap();
    let service = Arc::new(Service::new(id));

    let mut member_servers = Vec::new();
    let mut member_app_names = Vec::new();
    for i in 0..num_members {
        let member_app_name = Name::from_strings(["org", "ns", &format!("shared-member-{i}")]);
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
        let (app, notifications) = service
            .create_app(
                &member_app_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret),
            )
            .unwrap();
        let app = Arc::new(app);
        let server = Arc::new(Server::new_with_shared_responses(
            app.clone(),
            member_app_name.clone(),
            None,
            notifications,
            None,
        ));
        member_app_names.push(member_app_name);
        member_servers.push(server);
    }

    let client_name = Name::from_strings(["org", "ns", "shared-client"]);
    let secret = SharedSecret::new("client", TEST_VALID_SECRET).unwrap();
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let channel =
        Channel::new_with_members_shared(Arc::new(client_app), member_app_names, None)
            .expect("failed to create shared-responses channel");

    MulticastTestEnv {
        service,
        member_servers,
        channel,
    }
}

/// Collect at most `n` `PeerMessage`s from `peer_stream` within `timeout`.
async fn collect_peer_messages(
    peer_stream: &mut PeerResponseStream,
    n: usize,
    timeout: Duration,
) -> Vec<PeerMessage> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(timeout, async {
        for _ in 0..n {
            match peer_stream.next().await {
                Some(msg) => out.push(msg),
                None => break,
            }
        }
    })
    .await;
    out
}

// ============================================================================
// Test: shared-responses unary
// ============================================================================

/// Two servers opt in; client uses `new_with_members_shared`.
/// Each server handler collects the *other* server's response via `PeerResponseStream`,
/// while the client also collects both responses via the multicast stream.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_unary() {
    use std::sync::Mutex;

    const NUM_MEMBERS: usize = 2;
    let mut env = new_shared_env("test-shared-unary", NUM_MEMBERS).await;

    // Each server records the peer messages it received.
    let peer_payloads: Vec<Arc<Mutex<Vec<Vec<u8>>>>> = (0..NUM_MEMBERS)
        .map(|_| Arc::new(Mutex::new(Vec::new())))
        .collect();

    for (i, server) in env.member_servers.iter().enumerate() {
        let collected = peer_payloads[i].clone();
        server.register_unary_unary_shared(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context, mut peer: PeerResponseStream| {
                let collected = collected.clone();
                async move {
                    // Collect peer responses in the background so this handler can
                    // return its own response without deadlocking on the other server.
                    tokio::spawn(async move {
                        let peers = collect_peer_messages(
                            &mut peer,
                            NUM_MEMBERS - 1,
                            Duration::from_secs(5),
                        )
                        .await;
                        let mut guard = collected.lock().unwrap();
                        for p in peers {
                            guard.push(p.payload.clone());
                        }
                    });
                    Ok(TestResponse {
                        member_id: i,
                        result: format!("M{i}: {}", req.message),
                        count: req.value + i as i32,
                    })
                }
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
    let responses =
        collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "shared-unary").await;
    assert_eq!(responses.len(), NUM_MEMBERS);

    // Give handlers time to finish collecting peer messages.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Each server should have seen exactly one peer message.
    for (i, collected) in peer_payloads.iter().enumerate() {
        let guard = collected.lock().unwrap();
        assert_eq!(
            guard.len(),
            NUM_MEMBERS - 1,
            "server {i} saw {} peer messages, expected {}",
            guard.len(),
            NUM_MEMBERS - 1
        );
    }

    env.shutdown().await;
}

// ============================================================================
// Test: shared-responses stream
// ============================================================================

/// Two servers opt in with stream handlers.
/// Each server collects the other's response frames via `PeerResponseStream`.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_stream() {
    use std::sync::Mutex;

    const NUM_MEMBERS: usize = 2;
    let mut env = new_shared_env("test-shared-stream", NUM_MEMBERS).await;

    let peer_counts: Vec<Arc<Mutex<usize>>> = (0..NUM_MEMBERS)
        .map(|_| Arc::new(Mutex::new(0usize)))
        .collect();

    for (i, server) in env.member_servers.iter().enumerate() {
        let counter = peer_counts[i].clone();
        server.register_unary_stream_shared(
            "TestService",
            "StreamEcho",
            move |req: TestRequest, _ctx: Context, mut peer: PeerResponseStream| {
                let counter = counter.clone();
                async move {
                    let c = req.value as usize;
                    // Concurrently drain the peer stream.
                    tokio::spawn(async move {
                        let mut seen = 0usize;
                        while let Some(_msg) = peer.next().await {
                            seen += 1;
                        }
                        *counter.lock().unwrap() = seen;
                    });
                    // Return `c` response frames.
                    let responses: Vec<Result<TestResponse, RpcError>> = (0..c)
                        .map(|j| {
                            Ok(TestResponse {
                                member_id: i,
                                result: format!("M{i}:{j}"),
                                count: j as i32,
                            })
                        })
                        .collect();
                    Ok(stream::iter(responses))
                }
            },
        );
    }
    env.start_all_servers().await;

    let frames_each = 2usize;
    let stream = env.channel.multicast_unary_stream::<TestRequest, TestResponse>(
        "TestService",
        "StreamEcho",
        TestRequest {
            message: "stream".to_string(),
            value: frames_each as i32,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    let responses = collect_n_multicast(
        stream,
        NUM_MEMBERS * frames_each,
        Duration::from_secs(10),
        "shared-stream",
    )
    .await;
    assert_eq!(responses.len(), NUM_MEMBERS * frames_each);

    // Give peer-drain tasks time to finish.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Each server should have seen >= frames_each peer frames from the other server.
    for (i, counter) in peer_counts.iter().enumerate() {
        let seen = *counter.lock().unwrap();
        assert!(
            seen >= frames_each,
            "server {i} saw {seen} peer frames, expected >= {frames_each}"
        );
    }

    env.shutdown().await;
}

// ============================================================================
// Test: peer EOS does not close the request stream
// ============================================================================

/// Verify that the handler's `DecodedStream<Req>` remains open after the peer
/// sends its EOS — the two streams are completely independent.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_peer_eos_not_terminal() {
    const NUM_MEMBERS: usize = 2;
    let mut env = new_shared_env("test-shared-peer-eos", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_stream_unary_shared(
            "TestService",
            "Sum",
            move |mut req_stream: DecodedStream<TestRequest>,
                  _ctx: Context,
                  mut peer: PeerResponseStream| async move {
                let mut total = 0i32;
                while let Some(r) = req_stream.next().await {
                    let req = r?;
                    total += req.value;
                }
                // Drain peer stream in background so we can send our response
                // without deadlocking on the other server's peer EOS.
                tokio::spawn(async move {
                    while let Some(_) = peer.next().await {}
                });
                Ok(TestResponse {
                    member_id: i,
                    result: "sum".to_string(),
                    count: total,
                })
            },
        );
    }
    env.start_all_servers().await;

    let requests: Vec<TestRequest> = (1..=3)
        .map(|v| TestRequest {
            message: "n".to_string(),
            value: v,
        })
        .collect();
    let request_stream = stream::iter(requests);
    let result_stream = env
        .channel
        .multicast_stream_unary::<TestRequest, TestResponse>(
            "TestService",
            "Sum",
            request_stream,
            Some(Duration::from_secs(10)),
            None,
        );
    let responses =
        collect_n_multicast(result_stream, NUM_MEMBERS, Duration::from_secs(10), "peer-eos").await;
    assert_eq!(responses.len(), NUM_MEMBERS);
    for r in &responses {
        assert_eq!(r.message.count, 6, "server {} returned wrong sum", r.message.member_id);
    }

    env.shutdown().await;
}

// ============================================================================
// Test: server rejects shared-responses session
// ============================================================================

/// One server opts in (`accept_shared_responses = true`), the other does not.
/// The non-opting server should return a `failed_precondition` error while the
/// opting server succeeds.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_server_rejects() {
    use slim_rpc::RpcCode;

    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-shared-reject").unwrap();
    let service = Arc::new(Service::new(id));

    let make_app = |app_name: &Name| {
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
        let (app, notifications) = service
            .create_app(
                app_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret),
            )
            .unwrap();
        (Arc::new(app), notifications)
    };

    // Member 0 — accepts shared-responses.
    let name0 = Name::from_strings(["org", "ns", "reject-member-0"]);
    let (app0, notif0) = make_app(&name0);
    let server0 = Arc::new(Server::new_with_shared_responses(
        app0.clone(),
        name0.clone(),
        None,
        notif0,
        None,
    ));
    server0.register_unary_unary_shared(
        "TestService",
        "Echo",
        |req: TestRequest, _ctx: Context, _peer: PeerResponseStream| async move {
            Ok(TestResponse {
                member_id: 0,
                result: req.message,
                count: 0,
            })
        },
    );

    // Member 1 — standard server, does NOT accept shared-responses.
    let name1 = Name::from_strings(["org", "ns", "reject-member-1"]);
    let (app1, notif1) = make_app(&name1);
    let server1 = Arc::new(Server::new(app1.clone(), name1.clone(), notif1));
    server1.register_unary_unary(
        "TestService",
        "Echo",
        |req: TestRequest, _ctx: Context| async move {
            Ok(TestResponse {
                member_id: 1,
                result: req.message,
                count: 0,
            })
        },
    );

    let s0 = server0.clone();
    tokio::spawn(async move { s0.serve().await });
    let s1 = server1.clone();
    tokio::spawn(async move { s1.serve().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client_name = Name::from_strings(["org", "ns", "reject-client"]);
    let secret = SharedSecret::new("client", TEST_VALID_SECRET).unwrap();
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let channel =
        Channel::new_with_members_shared(Arc::new(client_app), vec![name0, name1], None)
            .expect("channel creation ok");

    let stream = channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "test".to_string(),
            value: 0,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    // Collect 2 items: one success (server0) and one error (server1).
    let results =
        collect_n_mixed(stream, 2, Duration::from_secs(10), "reject-test").await;
    assert_eq!(results.len(), 2);
    let errors: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(successes.len(), 1, "expected 1 success");
    assert_eq!(errors.len(), 1, "expected 1 error");
    let err = errors[0].as_ref().unwrap_err();
    assert_eq!(
        err.code(),
        RpcCode::FailedPrecondition,
        "rejecting server must return failed_precondition, got {:?}",
        err.code()
    );

    channel.close(None).await.ok();
    server0.shutdown().await;
    server1.shutdown().await;
    service.shutdown().await.unwrap();
}

// ============================================================================
// Test: shared-responses default off (regression)
// ============================================================================

/// Standard GROUP channel + standard servers (no opt-in).
/// No peer messages should arrive; existing behavior unchanged.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_default_off() {
    const NUM_MEMBERS: usize = 2;
    let mut env = MulticastTestEnv::new("test-shared-default-off", NUM_MEMBERS).await;

    for (i, server) in env.member_servers.iter().enumerate() {
        server.register_unary_unary(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context| async move {
                Ok(TestResponse {
                    member_id: i,
                    result: req.message,
                    count: req.value,
                })
            },
        );
    }
    env.start_all_servers().await;

    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "normal".to_string(),
            value: 1,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    let responses = collect_n_multicast(
        stream,
        NUM_MEMBERS,
        Duration::from_secs(10),
        "default-off",
    )
    .await;
    assert_eq!(responses.len(), NUM_MEMBERS);
    for r in &responses {
        assert_eq!(r.message.result, "normal");
    }

    env.shutdown().await;
}

// ============================================================================
// Test: shared-responses with 3 servers (N>2 regression)
// ============================================================================

/// Three servers each register a unary-unary-shared handler.
/// Each server should receive exactly 2 peer messages (one from each other server).
/// This is a regression test for the single-EOS-closes bug that affected N>2 groups.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multicast_shared_responses_three_servers() {
    use std::sync::Mutex;

    const NUM_MEMBERS: usize = 3;
    let mut env = new_shared_env("test-shared-three", NUM_MEMBERS).await;

    let peer_payloads: Vec<Arc<Mutex<Vec<Vec<u8>>>>> = (0..NUM_MEMBERS)
        .map(|_| Arc::new(Mutex::new(Vec::new())))
        .collect();

    for (i, server) in env.member_servers.iter().enumerate() {
        let collected = peer_payloads[i].clone();
        server.register_unary_unary_shared(
            "TestService",
            "Echo",
            move |req: TestRequest, _ctx: Context, mut peer: PeerResponseStream| {
                let collected = collected.clone();
                async move {
                    // Collect peer responses in background so no deadlock.
                    tokio::spawn(async move {
                        let peers = collect_peer_messages(
                            &mut peer,
                            NUM_MEMBERS - 1,
                            Duration::from_secs(5),
                        )
                        .await;
                        let mut guard = collected.lock().unwrap();
                        for p in peers {
                            guard.push(p.payload.clone());
                        }
                    });
                    Ok(TestResponse {
                        member_id: i,
                        result: format!("M{i}: {}", req.message),
                        count: req.value + i as i32,
                    })
                }
            },
        );
    }
    env.start_all_servers().await;

    let stream = env.channel.multicast_unary::<TestRequest, TestResponse>(
        "TestService",
        "Echo",
        TestRequest {
            message: "three".to_string(),
            value: 1,
        },
        Some(Duration::from_secs(10)),
        None,
    );
    let responses =
        collect_n_multicast(stream, NUM_MEMBERS, Duration::from_secs(10), "three-servers").await;
    assert_eq!(responses.len(), NUM_MEMBERS, "client should receive all 3 responses");

    // Give background peer-collection tasks time to finish.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Each server should have received exactly 2 peer messages (one from each other server).
    for (i, collected) in peer_payloads.iter().enumerate() {
        let guard = collected.lock().unwrap();
        assert_eq!(
            guard.len(),
            NUM_MEMBERS - 1,
            "server {i} saw {} peer messages, expected {}",
            guard.len(),
            NUM_MEMBERS - 1
        );
    }

    env.shutdown().await;
}
