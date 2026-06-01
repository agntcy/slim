// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for peer-to-peer subscription synchronization.
//!
//! Verifies that SLIM replicas configured as peers:
//! 1. Form a full mesh with exactly one connection per peer pair
//! 2. Propagate subscriptions to all peers (but not remote connections)
//! 3. Route messages correctly across peers (app on peer A → app on peer B)

use std::net::TcpListener;
use std::time::Duration;

use rstest::rstest;
use slim_auth::shared_secret::SharedSecret;
use slim_config::client::ClientConfig;
use slim_config::component::id::ID;
use slim_config::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::{ApplicationPayload, ProtoMessage as Message, ProtoName as Name};
use slim_datapath::peer_discovery::{PeerConfig, PeerTopology, StaticPeerEntry};
use slim_datapath::tables::{ConnType, SubscriptionTable};
use slim_service::{Service, ServiceConfiguration};
use slim_testing::utils::TEST_VALID_SECRET;

// ============================================================================
// Helpers
// ============================================================================

fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test port");
    let port = listener.local_addr().expect("failed to read port").port();
    drop(listener);
    port
}

/// Wait for a TCP endpoint to become reachable.
async fn wait_for_server(addr: &str, max_attempts: u32) {
    for attempt in 0..max_attempts {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => return,
            Err(_) => {
                if attempt < max_attempts - 1 {
                    let backoff = Duration::from_millis(50 * (1 + attempt as u64).min(10));
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    panic!("server at {addr} did not become ready");
}

struct PeerNode {
    service: Service,
    port: u16,
    self_id: String,
}

/// Build a set of peer nodes with the given topology.
/// Each node gets a server on a unique port, and static_peers listing all other nodes.
fn build_peer_configs(count: usize) -> Vec<(u16, String, PeerConfig)> {
    build_peer_configs_with_topology(count, PeerTopology::FullMesh)
}

fn build_peer_configs_with_topology(
    count: usize,
    topology: PeerTopology,
) -> Vec<(u16, String, PeerConfig)> {
    let ports: Vec<u16> = (0..count).map(|_| reserve_port()).collect();
    // Use port-based node IDs for deterministic tie-breaking in tests
    let node_ids: Vec<String> = (0..count).map(|i| format!("peer-{}", ports[i])).collect();

    // Static peers: all endpoints (including self — will be filtered by node_id)
    let static_peers: Vec<StaticPeerEntry> = ports
        .iter()
        .zip(node_ids.iter())
        .map(|(&port, node_id)| StaticPeerEntry {
            node_id: node_id.clone(),
            config: ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port}"))
                .with_tls_setting(TlsClientConfig::default().with_insecure(true)),
        })
        .collect();

    let peer_config = PeerConfig {
        peer_group: "test-group".to_string(),
        topology,
        static_peers,
        discovery: None,
    };

    let mut configs = Vec::new();
    for i in 0..count {
        configs.push((ports[i], node_ids[i].clone(), peer_config.clone()));
    }

    configs
}

/// Start a peer node with the given configuration.
async fn start_peer_node(port: u16, node_id: String, peer_config: PeerConfig) -> PeerNode {
    let server_config = ServerConfig::with_endpoint(&format!("127.0.0.1:{port}"))
        .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let mut service_config = ServiceConfiguration::new();
    service_config.node_id = node_id.clone();
    let service_config = service_config
        .with_dataplane_server(vec![server_config])
        .with_peers(peer_config);

    let svc_id = ID::new_with_str(&format!("slim/{node_id}")).unwrap();
    let service = service_config.build_server(svc_id).unwrap();
    service.run().await.expect("failed to start peer node");

    PeerNode {
        service,
        port,
        self_id: node_id,
    }
}

/// Wait until the expected number of peer connections appear on a node.
async fn wait_for_peer_connections(service: &Service, expected: usize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let mut peer_count = 0;
        service
            .message_processor()
            .connection_table()
            .for_each(|_id, conn| {
                if conn.connection_type() == ConnType::Peer {
                    peer_count += 1;
                }
            });

        if peer_count >= expected {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for {expected} peer connections, got {peer_count}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Count peer connections on a node.
fn count_peer_connections(service: &Service) -> usize {
    let mut count = 0;
    service
        .message_processor()
        .connection_table()
        .for_each(|_id, conn| {
            if conn.connection_type() == ConnType::Peer {
                count += 1;
            }
        });
    count
}

// ============================================================================
// Tests
// ============================================================================

/// Test that 3 peer nodes form a full mesh with exactly 1 connection per pair.
/// A full mesh of 3 nodes = 3 connections total (A-B, A-C, B-C).
#[tokio::test(flavor = "multi_thread")]
async fn test_peer_mesh_formation() {
    let configs = build_peer_configs(3);

    // Start all nodes
    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        // Wait for server to be ready before starting next node
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for peer connections to establish on all nodes.
    // Each node should have exactly 2 peer connections (one to each other node).
    for node in &nodes {
        wait_for_peer_connections(&node.service, 2, Duration::from_secs(10)).await;
    }

    // Verify exactly 2 peer connections per node (no duplicates)
    for node in &nodes {
        let peer_conns = count_peer_connections(&node.service);
        assert_eq!(
            peer_conns, 2,
            "node {} should have exactly 2 peer connections, got {}",
            node.self_id, peer_conns
        );
    }

    // Shutdown all nodes
    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that a subscription created on one peer propagates to all other peers
/// but NOT to remote connections.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_propagation_to_peers() {
    let configs = build_peer_configs(3);

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for full mesh
    for node in &nodes {
        wait_for_peer_connections(&node.service, 2, Duration::from_secs(10)).await;
    }

    // Allow peer sync to fully initialize
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create an app on node 0 (auto-subscribes to its name)
    let app_name = Name::from_strings(["org", "ns", "my-service"]).with_id(1);
    let (_app, _app_rx) = nodes[0]
        .service
        .create_app(
            &app_name,
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
        )
        .expect("failed to create app on node 0");

    // Wait for subscription to propagate to peers
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check that nodes 1 and 2 have the subscription in their tables
    // with peer connection type
    let prefix = Name::from_strings(["org", "ns", "my-service"]);
    for (i, node) in nodes.iter().enumerate().skip(1) {
        let sub_table = node.service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local_conns, _remote_conns, peer_conns| {
            if name == &prefix && !peer_conns.is_empty() {
                found = true;
            }
        });
        assert!(
            found,
            "node {i} should have the subscription from node 0 under peer connections"
        );
    }

    // Verify the originating node has it under local connections
    let sub_table = nodes[0].service.message_processor().subscription_table();
    let mut found_local = false;
    sub_table.for_each(|name, _id, local_conns, _remote_conns, _peer_conns| {
        if name == &prefix && !local_conns.is_empty() {
            found_local = true;
        }
    });
    assert!(
        found_local,
        "node 0 should have the subscription under local connections"
    );

    // Shutdown
    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that a message published on one peer reaches a subscriber on another peer.
#[tokio::test(flavor = "multi_thread")]
async fn test_message_delivery_across_peers() {
    let configs = build_peer_configs(2);

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for peer connection
    for node in &nodes {
        wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let topic = Name::from_strings(["org", "ns", "chat"]).with_id(42);

    // Register subscriber on node 1 (raw connection to verify message receipt)
    let (_sub_conn_id, sub_tx, mut sub_rx) = nodes[1]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register subscriber connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for subscription to propagate to node 0 via peer sync
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Register publisher on node 0 (raw connection for sending)
    let (_pub_conn_id, pub_tx, _pub_rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register publisher connection");

    // Publish a message
    let payload = ApplicationPayload::new("test", b"hello from peer 0".to_vec()).as_content();
    let pub_msg = Message::builder()
        .source(Name::from_strings(["org", "ns", "publisher"]).with_id(99))
        .destination(topic.clone())
        .payload(payload)
        .build_publish()
        .unwrap();
    pub_tx.send(Ok(pub_msg)).await.unwrap();

    // Wait for message delivery on node 1
    let received = tokio::time::timeout(Duration::from_secs(5), sub_rx.recv())
        .await
        .expect("timeout waiting for message on subscriber")
        .expect("subscriber channel closed");

    let msg = received.expect("received error instead of message");
    assert!(
        msg.is_publish(),
        "expected a publish message, got something else"
    );

    // Shutdown
    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that subscriptions are NOT propagated to remote connections, only to peers.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_not_propagated_to_remote() {
    // Create 2 peer nodes + 1 remote node connected to peer node 0
    let peer_configs = build_peer_configs(2);

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in peer_configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Start a "remote" node that connects to node 0 as a regular (non-peer) client
    let remote_port = reserve_port();
    let remote_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{remote_port}"))
        .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    // Remote node connects to node 0 as a regular client (connection_type = Remote)
    let remote_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", nodes[0].port))
        .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let remote_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![remote_server])
        .with_dataplane_client(vec![remote_client]);

    let remote_id = ID::new_with_str("slim/remote-node").unwrap();
    let remote_service = remote_config.build_server(remote_id).unwrap();
    remote_service.run().await.expect("failed to start remote");
    wait_for_server(&format!("127.0.0.1:{remote_port}"), 40).await;

    // Wait for peer connections
    for node in &nodes {
        wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Subscribe on node 0 using create_app
    let topic = Name::from_strings(["org", "ns", "peer-only"]);
    let (_app, _app_notif_rx) = nodes[0]
        .service
        .create_app(
            &topic,
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
        )
        .expect("failed to create app on node 0");

    // Wait for propagation
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify peer node 1 HAS the subscription (under peer conns)
    let prefix = &topic;
    let sub_table = nodes[1].service.message_processor().subscription_table();
    let mut found_on_peer = false;
    sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
        if name == prefix && !peer_conns.is_empty() {
            found_on_peer = true;
        }
    });
    assert!(
        found_on_peer,
        "peer node 1 should have the subscription from node 0"
    );

    // Verify remote node does NOT have the subscription
    let remote_sub_table = remote_service.message_processor().subscription_table();
    let mut found_on_remote = false;
    remote_sub_table.for_each(|name, _id, _local, _remote, _peer| {
        if name == prefix {
            found_on_remote = true;
        }
    });
    assert!(
        !found_on_remote,
        "remote node should NOT have the peer subscription"
    );

    // Shutdown
    for node in &nodes {
        node.service.shutdown().await.ok();
    }
    remote_service.shutdown().await.ok();
}

// ============================================================================
// Hub-and-Spoke Topology Tests
// ============================================================================

/// Test that hub-and-spoke topology creates only hub-to-spoke connections.
/// With 3 nodes, the hub connects to 2 spokes (2 connections total per spoke perspective,
/// but only 1 on each spoke since they only connect to the hub).
#[tokio::test(flavor = "multi_thread")]
async fn test_hub_spoke_topology_connections() {
    let configs = build_peer_configs_with_topology(3, PeerTopology::HubAndSpoke);

    // Determine which node will be the hub (smallest node_id)
    let hub_idx = configs
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, self_id, _))| self_id.clone())
        .map(|(i, _)| i)
        .unwrap();

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for connections to establish.
    // Hub should have 2 peer connections (one to each spoke).
    // Each spoke should have 1 peer connection (to the hub only).
    wait_for_peer_connections(&nodes[hub_idx].service, 2, Duration::from_secs(10)).await;
    for (i, node) in nodes.iter().enumerate() {
        if i != hub_idx {
            wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
        }
    }

    // Verify connection counts
    let hub_conns = count_peer_connections(&nodes[hub_idx].service);
    assert_eq!(hub_conns, 2, "hub should have 2 peer connections");

    for (i, node) in nodes.iter().enumerate() {
        if i != hub_idx {
            let spoke_conns = count_peer_connections(&node.service);
            assert_eq!(
                spoke_conns, 1,
                "spoke {} should have 1 peer connection (hub only), got {}",
                node.self_id, spoke_conns
            );
        }
    }

    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that subscriptions propagate through the hub to all spokes.
/// Spoke A subscribes → hub relays to spoke B.
#[tokio::test(flavor = "multi_thread")]
async fn test_hub_spoke_subscription_relay() {
    let configs = build_peer_configs_with_topology(3, PeerTopology::HubAndSpoke);

    // Determine hub index (smallest node_id)
    let hub_idx = configs
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, self_id, _))| self_id.clone())
        .map(|(i, _)| i)
        .unwrap();

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for hub to connect to all spokes
    wait_for_peer_connections(&nodes[hub_idx].service, 2, Duration::from_secs(10)).await;
    for (i, node) in nodes.iter().enumerate() {
        if i != hub_idx {
            wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Pick a spoke to subscribe on (not the hub)
    let spoke_a = if hub_idx == 0 { 1 } else { 0 };
    let spoke_b = (0..3).find(|&i| i != hub_idx && i != spoke_a).unwrap();

    // Create app on spoke_a and subscribe
    let topic = Name::from_strings(["org", "ns", "hub-test"]);
    let (app, _app_notif_rx) = nodes[spoke_a]
        .service
        .create_app(
            &topic,
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("app", TEST_VALID_SECRET).unwrap(),
        )
        .expect("failed to create app on spoke_a");

    app.subscribe(&topic, None).await.unwrap();

    // Wait for subscription to propagate: spoke_a → hub → spoke_b
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Verify hub has the subscription (from spoke_a via peer conn)
    let prefix = Name::from_strings(["org", "ns", "hub-test"]);
    let hub_table = nodes[hub_idx]
        .service
        .message_processor()
        .subscription_table();
    let mut found_on_hub = false;
    hub_table.for_each(|name, _id, _local, _remote, peer_conns| {
        if name == &prefix && !peer_conns.is_empty() {
            found_on_hub = true;
        }
    });
    assert!(
        found_on_hub,
        "hub should have the subscription from spoke_a"
    );

    // Verify spoke_b also has the subscription (relayed by hub)
    let spoke_b_table = nodes[spoke_b]
        .service
        .message_processor()
        .subscription_table();
    let mut found_on_spoke_b = false;
    spoke_b_table.for_each(|name, _id, _local, _remote, peer_conns| {
        if name == &prefix && !peer_conns.is_empty() {
            found_on_spoke_b = true;
        }
    });
    assert!(
        found_on_spoke_b,
        "spoke_b should have the subscription relayed by hub from spoke_a"
    );

    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that messages route correctly through the hub in hub-and-spoke topology.
/// App on spoke A publishes → hub relays → subscriber on spoke B receives.
#[tokio::test(flavor = "multi_thread")]
async fn test_hub_spoke_message_delivery() {
    let configs = build_peer_configs_with_topology(3, PeerTopology::HubAndSpoke);

    let hub_idx = configs
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, self_id, _))| self_id.clone())
        .map(|(i, _)| i)
        .unwrap();

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for topology
    wait_for_peer_connections(&nodes[hub_idx].service, 2, Duration::from_secs(10)).await;
    for (i, node) in nodes.iter().enumerate() {
        if i != hub_idx {
            wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let spoke_a = if hub_idx == 0 { 1 } else { 0 };
    let spoke_b = (0..3).find(|&i| i != hub_idx && i != spoke_a).unwrap();

    let topic = Name::from_strings(["org", "ns", "spoke-msg"]).with_id(10);

    // Register subscriber on spoke_b (raw connection to verify message receipt)
    let (_sub_conn, sub_tx, mut sub_rx) = nodes[spoke_b]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register subscriber");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for subscription to propagate: spoke_b → hub → spoke_a
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Register publisher on spoke_a (raw connection for sending)
    let (_pub_conn, pub_tx, _pub_rx) = nodes[spoke_a]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register publisher");

    // Publish
    let payload = ApplicationPayload::new("test", b"hello via hub".to_vec()).as_content();
    let pub_msg = Message::builder()
        .source(Name::from_strings(["org", "ns", "sender"]).with_id(1))
        .destination(topic.clone())
        .payload(payload)
        .build_publish()
        .unwrap();
    pub_tx.send(Ok(pub_msg)).await.unwrap();

    // Verify message arrives on spoke_b
    let received = tokio::time::timeout(Duration::from_secs(5), sub_rx.recv())
        .await
        .expect("timeout waiting for message on spoke_b subscriber")
        .expect("subscriber channel closed");

    let msg = received.expect("received error instead of message");
    assert!(msg.is_publish(), "expected publish message on spoke_b");

    for node in &nodes {
        node.service.shutdown().await.ok();
    }
}

// ============================================================================
// Topology-Parametrized Unsubscribe-on-Disconnect Tests
// ============================================================================

/// Starts `count` peer nodes with the given topology and waits for connections.
/// Returns the nodes ready for testing.
async fn start_topology(count: usize, topology: PeerTopology) -> Vec<PeerNode> {
    let configs = build_peer_configs_with_topology(count, topology.clone());

    let mut nodes = Vec::new();
    for (port, self_id, peer_config) in configs {
        let node = start_peer_node(port, self_id, peer_config).await;
        wait_for_server(&format!("127.0.0.1:{}", node.port), 40).await;
        nodes.push(node);
    }

    // Wait for the expected number of peer connections per topology
    match topology {
        PeerTopology::FullMesh => {
            let expected_per_node = count - 1;
            for node in &nodes {
                wait_for_peer_connections(
                    &node.service,
                    expected_per_node,
                    Duration::from_secs(10),
                )
                .await;
            }
        }
        PeerTopology::HubAndSpoke => {
            // Hub = smallest node_id, gets (count-1) connections; spokes get 1
            let hub_idx = nodes
                .iter()
                .enumerate()
                .min_by_key(|(_, n)| &n.self_id)
                .map(|(i, _)| i)
                .unwrap();
            wait_for_peer_connections(&nodes[hub_idx].service, count - 1, Duration::from_secs(10))
                .await;
            for (i, node) in nodes.iter().enumerate() {
                if i != hub_idx {
                    wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
                }
            }
        }
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
    nodes
}

/// Verify that a subscription for `prefix` exists on the peer connections of all nodes
/// except the one at `origin_idx`.
fn assert_subscription_on_peers(nodes: &[PeerNode], origin_idx: usize, prefix: &Name) {
    for (i, node) in nodes.iter().enumerate() {
        if i == origin_idx {
            continue;
        }
        let sub_table = node.service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
            if name == prefix && !peer_conns.is_empty() {
                found = true;
            }
        });
        assert!(
            found,
            "node {} should have the subscription from node {} under peer connections",
            node.self_id, nodes[origin_idx].self_id,
        );
    }
}

/// Verify that a subscription for `prefix` does NOT exist on the peer connections of
/// any node except the one at `origin_idx`.
fn assert_no_subscription_on_peers(nodes: &[PeerNode], origin_idx: usize, prefix: &Name) {
    for (i, node) in nodes.iter().enumerate() {
        if i == origin_idx {
            continue;
        }
        let sub_table = node.service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
            if name == prefix && !peer_conns.is_empty() {
                found = true;
            }
        });
        assert!(
            !found,
            "node {} should NOT have the subscription after unsubscribe from node {}",
            node.self_id, nodes[origin_idx].self_id,
        );
    }
}

async fn shutdown_nodes(nodes: &[PeerNode]) {
    for node in nodes {
        node.service.shutdown().await.ok();
    }
}

/// Test that when a local connection drops, the subscription is removed from peer nodes.
/// Verifies unsubscribe forwarding on connection close across topologies.
#[rstest]
#[case::full_mesh(PeerTopology::FullMesh)]
#[case::hub_and_spoke(PeerTopology::HubAndSpoke)]
#[tokio::test(flavor = "multi_thread")]
async fn test_unsubscribe_propagated_on_connection_drop(#[case] topology: PeerTopology) {
    let nodes = start_topology(3, topology).await;

    let topic = Name::from_strings(["org", "ns", "drop-test"]).with_id(77);

    // Register a local connection on node 0 and subscribe
    let (conn_id, sub_tx, _sub_rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(300001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for subscription to propagate to peers
    tokio::time::sleep(Duration::from_millis(500)).await;

    let prefix = Name::from_strings(["org", "ns", "drop-test"]);
    assert_subscription_on_peers(&nodes, 0, &prefix);

    // Drop the sender to simulate client disconnect
    drop(sub_tx);

    // Wait for the unsubscribe to propagate through peer sync
    tokio::time::sleep(Duration::from_millis(1000)).await;

    assert_no_subscription_on_peers(&nodes, 0, &prefix);

    // Also verify node 0 itself has no subscription
    let sub_table = nodes[0].service.message_processor().subscription_table();
    let mut found_on_origin = false;
    sub_table.for_each(|name, _id, local_conns, _remote, _peer| {
        if name == &prefix && !local_conns.is_empty() {
            found_on_origin = true;
        }
    });
    assert!(
        !found_on_origin,
        "node 0 should have removed the subscription locally after disconnect (conn_id={conn_id})"
    );

    shutdown_nodes(&nodes).await;
}

/// Test that batch subscribes (multiple sub_ids for the same name) still result in
/// correct unsubscribe forwarding to peers when the connection drops.
/// Verifies the forwarded_subs map in PeerSyncManager handles the sub_id correctly.
#[rstest]
#[case::full_mesh(PeerTopology::FullMesh)]
#[case::hub_and_spoke(PeerTopology::HubAndSpoke)]
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_subscribe_unsubscribe_on_disconnect(#[case] topology: PeerTopology) {
    let nodes = start_topology(2, topology).await;

    let topic = Name::from_strings(["org", "ns", "batch-test"]).with_id(42);

    // Register a local connection on node 0
    let (_conn_id, sub_tx, _sub_rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    // Send multiple subscribe messages for the same name (simulates SDK batch behavior)
    for i in 0..3u64 {
        let sub_msg = Message::builder()
            .source(topic.clone())
            .destination(topic.clone())
            .subscription_id(200001 + i)
            .build_subscribe()
            .unwrap();
        sub_tx.send(Ok(sub_msg)).await.unwrap();
    }

    // Wait for subscription to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    let prefix = Name::from_strings(["org", "ns", "batch-test"]);
    assert_subscription_on_peers(&nodes, 0, &prefix);

    // Drop the sender to simulate client disconnect
    drop(sub_tx);

    // Wait for unsubscribe propagation
    tokio::time::sleep(Duration::from_millis(1000)).await;

    assert_no_subscription_on_peers(&nodes, 0, &prefix);

    shutdown_nodes(&nodes).await;
}

/// Test that explicit unsubscribe (not connection drop) also propagates to peers.
#[rstest]
#[case::full_mesh(PeerTopology::FullMesh)]
#[case::hub_and_spoke(PeerTopology::HubAndSpoke)]
#[tokio::test(flavor = "multi_thread")]
async fn test_explicit_unsubscribe_propagated_to_peers(#[case] topology: PeerTopology) {
    let nodes = start_topology(2, topology).await;

    let topic = Name::from_strings(["org", "ns", "unsub-test"]).with_id(99);
    let sub_id: u64 = 400001;

    // Register a raw local connection on node 0
    let (_conn_id, sub_tx, _rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    // Subscribe
    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(sub_id)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for subscription to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    let prefix = Name::from_strings(["org", "ns", "unsub-test"]);
    assert_subscription_on_peers(&nodes, 0, &prefix);

    // Explicitly unsubscribe (while connection stays open)
    let unsub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(sub_id)
        .build_unsubscribe()
        .unwrap();
    sub_tx.send(Ok(unsub_msg)).await.unwrap();

    // Wait for unsubscribe propagation
    tokio::time::sleep(Duration::from_millis(1000)).await;

    assert_no_subscription_on_peers(&nodes, 0, &prefix);

    shutdown_nodes(&nodes).await;
}

/// Test that if two local connections subscribe to the same name on one node,
/// dropping one connection does NOT remove the subscription from peers (still reachable).
#[rstest]
#[case::full_mesh(PeerTopology::FullMesh)]
#[case::hub_and_spoke(PeerTopology::HubAndSpoke)]
#[tokio::test(flavor = "multi_thread")]
async fn test_partial_disconnect_does_not_remove_from_peers(#[case] topology: PeerTopology) {
    let nodes = start_topology(2, topology).await;

    let topic = Name::from_strings(["org", "ns", "multi-conn"]).with_id(55);

    // Register TWO local connections on node 0, both subscribing to the same name
    let (_conn1_id, sub_tx1, _rx1) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register connection 1");

    let (_conn2_id, sub_tx2, _rx2) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register connection 2");

    let sub_msg1 = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(100001)
        .build_subscribe()
        .unwrap();
    sub_tx1.send(Ok(sub_msg1)).await.unwrap();

    let sub_msg2 = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(100002)
        .build_subscribe()
        .unwrap();
    sub_tx2.send(Ok(sub_msg2)).await.unwrap();

    // Wait for subscription to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    let prefix = Name::from_strings(["org", "ns", "multi-conn"]);
    assert_subscription_on_peers(&nodes, 0, &prefix);

    // Drop only ONE connection — the name should still be reachable
    drop(sub_tx1);
    drop(_rx1);

    // Wait for any potential unsubscribe propagation
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify peers STILL have the subscription (second connection keeps it alive)
    assert_subscription_on_peers(&nodes, 0, &prefix);

    // Now drop the second connection — the name becomes fully unreachable
    drop(sub_tx2);
    drop(_rx2);
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // NOW the subscription should be gone from peers
    assert_no_subscription_on_peers(&nodes, 0, &prefix);

    shutdown_nodes(&nodes).await;
}
