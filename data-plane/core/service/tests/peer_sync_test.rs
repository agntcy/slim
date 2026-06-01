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

use slim_config::client::ClientConfig;
use slim_config::component::id::ID;
use slim_config::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::ApplicationPayload;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::api::ProtoName as Name;
use slim_datapath::peer_discovery::PeerConfig;
use slim_datapath::tables::{ConnType, SubscriptionTable};
use slim_service::{Service, ServiceConfiguration};

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

/// Build a set of peer nodes that form a full mesh.
/// Each node gets a server on a unique port, and static_peers listing all other nodes.
fn build_peer_configs(count: usize) -> Vec<(u16, String, PeerConfig)> {
    let ports: Vec<u16> = (0..count).map(|_| reserve_port()).collect();
    // Use port-based node IDs for deterministic tie-breaking in tests
    let node_ids: Vec<String> = (0..count).map(|i| format!("peer-{}", ports[i])).collect();

    let mut configs = Vec::new();
    for i in 0..count {
        // Static peers: all endpoints (including self — will be filtered by node_id)
        let static_peers: Vec<ClientConfig> = ports
            .iter()
            .map(|&port| {
                ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port}"))
                    .with_tls_setting(TlsClientConfig::default().with_insecure(true))
            })
            .collect();

        let peer_config = PeerConfig {
            peer_group: "test-group".to_string(),
            static_peers,
            discovery: None,
        };

        configs.push((ports[i], node_ids[i].clone(), peer_config));
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

    // Register a local app on node 0 and subscribe
    let topic = Name::from_strings(["org", "ns", "my-service"]).with_id(1);
    let (app_conn_id, app_tx, _app_rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection on node 0");

    // Send subscribe message
    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .build_subscribe()
        .unwrap();
    app_tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for subscription to propagate to peers
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check that nodes 1 and 2 have the subscription in their tables
    // with peer connection type
    let prefix = Name::from_strings(["org", "ns", "my-service"]);
    for (i, node) in nodes.iter().enumerate().skip(1) {
        let sub_table = node.service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local_conns, _remote_conns, peer_conns| {
            if name == &prefix {
                // On remote nodes, the subscription should appear under peer connections
                if !peer_conns.is_empty() {
                    found = true;
                }
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
        if name == &prefix && local_conns.contains(&app_conn_id) {
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

    // Register subscriber app on node 1
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

    // Register publisher app on node 0
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

    // Subscribe on node 0
    let topic = Name::from_strings(["org", "ns", "peer-only"]).with_id(7);
    let (_conn_id, tx, _rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .build_subscribe()
        .unwrap();
    tx.send(Ok(sub_msg)).await.unwrap();

    // Wait for propagation
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify peer node 1 HAS the subscription (under peer conns)
    let prefix = Name::from_strings(["org", "ns", "peer-only"]);
    let sub_table = nodes[1].service.message_processor().subscription_table();
    let mut found_on_peer = false;
    sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
        if name == &prefix && !peer_conns.is_empty() {
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
        if name == &prefix {
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
