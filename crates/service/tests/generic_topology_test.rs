// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for generic peer topology (TTL-based subscription forwarding).
//!
//! These tests verify that peers configured via normal dataplane clients/servers
//! (connection_type: peer) — without PeerSyncManager — correctly:
//! 1. Auto-register peer connections and perform full sync
//! 2. Forward subscriptions with TTL-based propagation depth
//! 3. Relay publish messages across multiple hops
//! 4. Propagate unsubscribes when connections drop

use std::time::Duration;

use slim_config::client::ClientConfig;
use slim_config::component::id::ID;
use slim_config::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::{ApplicationPayload, ProtoMessage as Message, ProtoName as Name};
use slim_datapath::tables::{ConnType, SubscriptionTable};
use slim_service::{Service, ServiceConfiguration};
use slim_testing::common::reserve_local_port;

// ============================================================================
// Helpers
// ============================================================================

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

#[allow(dead_code)]
struct GenericNode {
    service: Service,
    port: u16,
    node_id: String,
}

/// Build a linear chain topology: A → B → C
/// Each node has a server; each node connects as peer to the next node.
/// This tests multi-hop propagation (A's subscriptions should reach C via B).
async fn start_chain(count: usize) -> Vec<GenericNode> {
    let ports: Vec<u16> = (0..count).map(|_| reserve_local_port()).collect();
    let node_ids: Vec<String> = (0..count).map(|i| format!("chain-{}", ports[i])).collect();

    // Phase 1: Start all nodes with servers only (no clients).
    // Client connections are deferred because `run()` retries infinitely
    // when the target server isn't up yet, which would block.
    let mut nodes = Vec::new();
    for i in 0..count {
        let server_config = ServerConfig::with_endpoint(&format!("127.0.0.1:{}", ports[i]))
            .with_tls_settings(TlsServerConfig::default().with_insecure(true));

        let mut service_config = ServiceConfiguration::new();
        service_config.node_id = node_ids[i].clone();
        let service_config = service_config.with_dataplane_server(vec![server_config]);

        let svc_id = ID::new_with_str(&format!("slim/{}", node_ids[i])).unwrap();
        let service = service_config.build_server(svc_id).unwrap();
        service.run().await.expect("failed to start chain node");
        wait_for_server(&format!("127.0.0.1:{}", ports[i]), 40).await;

        nodes.push(GenericNode {
            service,
            port: ports[i],
            node_id: node_ids[i].clone(),
        });
    }

    // Phase 2: Connect each node to the NEXT node as a peer client.
    for i in 0..(count - 1) {
        let client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", ports[i + 1]))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true))
            .with_connection_type(ConnType::Peer);

        nodes[i]
            .service
            .connect(&client)
            .await
            .unwrap_or_else(|e| panic!("failed to connect node {i} to node {}: {e}", i + 1));
    }

    // Wait for peer connections to establish.
    // Interior nodes have 2 peers (one incoming, one outgoing).
    // End nodes have 1 peer each.
    for (i, node) in nodes.iter().enumerate() {
        let expected = match (i == 0, i == count - 1) {
            (true, true) => 0,   // single node
            (true, false) => 1,  // first node: only connects to next
            (false, true) => 1,  // last node: only receives from previous
            (false, false) => 2, // interior: one from prev + one to next
        };
        if expected > 0 {
            wait_for_peer_connections(&node.service, expected, Duration::from_secs(10)).await;
        }
    }

    tokio::time::sleep(Duration::from_millis(300)).await;
    nodes
}

/// Build a star topology: all nodes connect as peers to node 0 (center).
/// Unlike hub-and-spoke with PeerSyncManager, this uses generic TTL-based forwarding.
async fn start_star(count: usize) -> Vec<GenericNode> {
    let ports: Vec<u16> = (0..count).map(|_| reserve_local_port()).collect();
    let node_ids: Vec<String> = (0..count).map(|i| format!("star-{}", ports[i])).collect();

    let mut nodes = Vec::new();
    for i in 0..count {
        let server_config = ServerConfig::with_endpoint(&format!("127.0.0.1:{}", ports[i]))
            .with_tls_settings(TlsServerConfig::default().with_insecure(true));

        // Nodes 1..N connect as peer to node 0 (center).
        let mut peer_clients = Vec::new();
        if i > 0 {
            let client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", ports[0]))
                .with_tls_setting(TlsClientConfig::default().with_insecure(true))
                .with_connection_type(ConnType::Peer);
            peer_clients.push(client);
        }

        let mut service_config = ServiceConfiguration::new();
        service_config.node_id = node_ids[i].clone();
        let service_config = service_config
            .with_dataplane_server(vec![server_config])
            .with_dataplane_client(peer_clients);

        let svc_id = ID::new_with_str(&format!("slim/{}", node_ids[i])).unwrap();
        let service = service_config.build_server(svc_id).unwrap();
        service.run().await.expect("failed to start star node");
        wait_for_server(&format!("127.0.0.1:{}", ports[i]), 40).await;

        nodes.push(GenericNode {
            service,
            port: ports[i],
            node_id: node_ids[i].clone(),
        });
    }

    // Center node should have (count-1) peer connections (all spokes).
    // Each spoke has 1 peer connection (to center).
    if count > 1 {
        wait_for_peer_connections(&nodes[0].service, count - 1, Duration::from_secs(10)).await;
        for node in nodes.iter().skip(1) {
            wait_for_peer_connections(&node.service, 1, Duration::from_secs(10)).await;
        }
    }

    tokio::time::sleep(Duration::from_millis(300)).await;
    nodes
}

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

/// Wait until a subscription prefix exists under peer connections on the given node.
async fn wait_for_subscription(service: &Service, prefix: &Name, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let sub_table = service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
            if name == prefix && !peer_conns.is_empty() {
                found = true;
            }
        });
        if found {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait until a subscription prefix does NOT exist under peer connections on the given node.
async fn wait_for_no_subscription(service: &Service, prefix: &Name, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let sub_table = service.message_processor().subscription_table();
        let mut found = false;
        sub_table.for_each(|name, _id, _local, _remote, peer_conns| {
            if name == prefix && !peer_conns.is_empty() {
                found = true;
            }
        });
        if !found {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn shutdown_nodes(nodes: &[GenericNode]) {
    for node in nodes {
        node.service.shutdown().await.ok();
    }
}

// ============================================================================
// Tests: Star Topology (Generic — No PeerSyncManager)
// ============================================================================

/// Test that peers connected via normal config auto-register and form connections.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_star_peer_connections() {
    let nodes = start_star(3).await;

    // Center should have 2 peer connections, spokes 1 each
    let mut center_peers = 0;
    nodes[0]
        .service
        .message_processor()
        .connection_table()
        .for_each(|_id, conn| {
            if conn.connection_type() == ConnType::Peer {
                center_peers += 1;
            }
        });
    assert_eq!(center_peers, 2, "center should have 2 peer connections");

    for node in nodes.iter().skip(1) {
        let mut spoke_peers = 0;
        node.service
            .message_processor()
            .connection_table()
            .for_each(|_id, conn| {
                if conn.connection_type() == ConnType::Peer {
                    spoke_peers += 1;
                }
            });
        assert_eq!(
            spoke_peers, 1,
            "spoke {} should have 1 peer connection",
            node.node_id
        );
    }

    shutdown_nodes(&nodes).await;
}

/// Test subscription propagation through center in star topology.
/// Spoke 1 subscribes → center relays → spoke 2 receives the subscription.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_star_subscription_relay() {
    let nodes = start_star(3).await;

    let topic = Name::from_strings(["org", "ns", "star-sub"]).with_id(1);

    // Register subscriber on spoke 1 (node index 1)
    let (_conn_id, sub_tx, _rx) = nodes[1]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(500001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    let prefix = Name::from_strings(["org", "ns", "star-sub"]);

    // Center (node 0) should have the subscription via peer
    assert!(
        wait_for_subscription(&nodes[0].service, &prefix, Duration::from_secs(3)).await,
        "center should have subscription from spoke 1"
    );

    // Spoke 2 (node 2) should also have it (relayed by center)
    assert!(
        wait_for_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "spoke 2 should have subscription relayed by center from spoke 1"
    );

    shutdown_nodes(&nodes).await;
}

/// Test message delivery through center in star topology.
/// Publisher on spoke 1 → center → subscriber on spoke 2.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_star_message_delivery() {
    let nodes = start_star(3).await;

    let topic = Name::from_strings(["org", "ns", "star-msg"]).with_id(10);

    // Register subscriber on spoke 2 (node 2)
    let (_sub_conn, sub_tx, mut sub_rx) = nodes[2]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register subscriber");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(600001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Consume the subscription ACK before waiting for publish
    let ack = tokio::time::timeout(Duration::from_secs(3), sub_rx.recv())
        .await
        .expect("timeout waiting for subscription ack")
        .expect("subscriber channel closed")
        .expect("received error");
    assert!(ack.is_subscription_ack(), "expected subscription ack");

    // Wait for subscription to propagate: spoke 2 → center → spoke 1
    let prefix = Name::from_strings(["org", "ns", "star-msg"]);
    assert!(
        wait_for_subscription(&nodes[1].service, &prefix, Duration::from_secs(3)).await,
        "spoke 1 should have subscription from spoke 2"
    );

    // Register publisher on spoke 1 (node 1)
    let (_pub_conn, pub_tx, _pub_rx) = nodes[1]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register publisher");

    // Publish a message
    let payload = ApplicationPayload::new("test", b"hello via center".to_vec()).as_content();
    let pub_msg = Message::builder()
        .source(Name::from_strings(["org", "ns", "sender"]).with_id(1))
        .destination(topic.clone())
        .payload(payload)
        .build_publish()
        .unwrap();
    pub_tx.send(Ok(pub_msg)).await.unwrap();

    // Verify message arrives on spoke 2
    let received = tokio::time::timeout(Duration::from_secs(5), sub_rx.recv())
        .await
        .expect("timeout waiting for message on spoke 2")
        .expect("subscriber channel closed");

    let msg = received.expect("received error instead of message");
    assert!(msg.is_publish(), "expected publish message on spoke 2");

    shutdown_nodes(&nodes).await;
}

/// Test unsubscribe propagation through center when a local connection drops.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_star_unsubscribe_on_disconnect() {
    let nodes = start_star(3).await;

    let topic = Name::from_strings(["org", "ns", "star-drop"]).with_id(77);

    // Register and subscribe on spoke 1
    let (_conn_id, sub_tx, _rx) = nodes[1]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(700001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    let prefix = Name::from_strings(["org", "ns", "star-drop"]);

    // Wait for subscription to propagate to center and spoke 2
    assert!(
        wait_for_subscription(&nodes[0].service, &prefix, Duration::from_secs(3)).await,
        "center should have subscription"
    );
    assert!(
        wait_for_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "spoke 2 should have subscription"
    );

    // Drop the local connection on spoke 1 (triggers unsubscribe)
    drop(sub_tx);

    // Unsubscribe should propagate: spoke 1 → center → spoke 2
    assert!(
        wait_for_no_subscription(&nodes[0].service, &prefix, Duration::from_secs(3)).await,
        "center should have removed subscription after spoke 1 disconnected"
    );
    assert!(
        wait_for_no_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "spoke 2 should have removed subscription after spoke 1 disconnected"
    );

    shutdown_nodes(&nodes).await;
}

// ============================================================================
// Tests: Chain Topology (Multi-Hop)
// ============================================================================

/// Test that subscriptions propagate across a 3-node chain: A → B → C.
/// A subscription on A should reach C via B (2 hops).
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_chain_subscription_multi_hop() {
    let nodes = start_chain(3).await;

    let topic = Name::from_strings(["org", "ns", "chain-sub"]).with_id(1);

    // Subscribe on node A (index 0)
    let (_conn_id, sub_tx, _rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(800001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    let prefix = Name::from_strings(["org", "ns", "chain-sub"]);

    // Node B (index 1) should have it (1 hop)
    assert!(
        wait_for_subscription(&nodes[1].service, &prefix, Duration::from_secs(3)).await,
        "node B should have subscription from node A (1 hop)"
    );

    // Node C (index 2) should also have it (2 hops via B relay)
    assert!(
        wait_for_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "node C should have subscription from node A (2 hops via B)"
    );

    shutdown_nodes(&nodes).await;
}

/// Test message delivery across a 3-node chain.
/// Subscriber on A, publisher on C → message travels C → B → A.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_chain_message_delivery() {
    let nodes = start_chain(3).await;

    let topic = Name::from_strings(["org", "ns", "chain-msg"]).with_id(20);

    // Register subscriber on node A (index 0)
    let (_sub_conn, sub_tx, mut sub_rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register subscriber on A");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(900001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    // Consume the subscription ACK before waiting for publish
    let ack = tokio::time::timeout(Duration::from_secs(3), sub_rx.recv())
        .await
        .expect("timeout waiting for subscription ack")
        .expect("subscriber channel closed")
        .expect("received error");
    assert!(ack.is_subscription_ack(), "expected subscription ack");

    // Wait for subscription to propagate to C (through B)
    let prefix = Name::from_strings(["org", "ns", "chain-msg"]);
    assert!(
        wait_for_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "node C should have subscription from node A"
    );

    // Register publisher on node C (index 2)
    let (_pub_conn, pub_tx, _pub_rx) = nodes[2]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register publisher on C");

    // Publish from C
    let payload = ApplicationPayload::new("test", b"hello from C".to_vec()).as_content();
    let pub_msg = Message::builder()
        .source(Name::from_strings(["org", "ns", "pub-c"]).with_id(1))
        .destination(topic.clone())
        .payload(payload)
        .build_publish()
        .unwrap();
    pub_tx.send(Ok(pub_msg)).await.unwrap();

    // Verify message arrives on node A (through B)
    let received = tokio::time::timeout(Duration::from_secs(5), sub_rx.recv())
        .await
        .expect("timeout waiting for message on node A")
        .expect("subscriber channel closed");

    let msg = received.expect("received error instead of message");
    assert!(msg.is_publish(), "expected publish message on node A");

    shutdown_nodes(&nodes).await;
}

/// Test that unsubscribes propagate across the chain on connection drop.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_chain_unsubscribe_on_disconnect() {
    let nodes = start_chain(3).await;

    let topic = Name::from_strings(["org", "ns", "chain-drop"]).with_id(33);

    // Subscribe on node A
    let (_conn_id, sub_tx, _rx) = nodes[0]
        .service
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register local connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(1000001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();

    let prefix = Name::from_strings(["org", "ns", "chain-drop"]);

    // Wait for subscription to reach node C
    assert!(
        wait_for_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "node C should have subscription"
    );

    // Drop connection on A
    drop(sub_tx);

    // Unsubscribe should propagate A → B → C
    assert!(
        wait_for_no_subscription(&nodes[1].service, &prefix, Duration::from_secs(3)).await,
        "node B should have removed subscription after A disconnected"
    );
    assert!(
        wait_for_no_subscription(&nodes[2].service, &prefix, Duration::from_secs(3)).await,
        "node C should have removed subscription after A disconnected"
    );

    shutdown_nodes(&nodes).await;
}

/// Test that full sync works when a new peer connects to an existing node
/// that already has subscriptions.
#[tokio::test(flavor = "multi_thread")]
async fn test_generic_full_sync_on_late_join() {
    // Start just node A with a server
    let port_a = reserve_local_port();
    let port_b = reserve_local_port();

    let server_a = ServerConfig::with_endpoint(&format!("127.0.0.1:{port_a}"))
        .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let mut config_a = ServiceConfiguration::new();
    config_a.node_id = "late-join-a".to_string();
    let config_a = config_a.with_dataplane_server(vec![server_a]);

    let svc_id_a = ID::new_with_str("slim/late-join-a").unwrap();
    let service_a = config_a.build_server(svc_id_a).unwrap();
    service_a.run().await.expect("failed to start node A");
    wait_for_server(&format!("127.0.0.1:{port_a}"), 40).await;

    // Subscribe on node A BEFORE node B joins
    let topic = Name::from_strings(["org", "ns", "late-join"]).with_id(5);
    let (_conn_id, sub_tx, _rx) = service_a
        .message_processor()
        .register_local_connection(false)
        .expect("failed to register connection");

    let sub_msg = Message::builder()
        .source(topic.clone())
        .destination(topic.clone())
        .subscription_id(1100001)
        .build_subscribe()
        .unwrap();
    sub_tx.send(Ok(sub_msg)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now start node B connecting as peer to A
    let server_b = ServerConfig::with_endpoint(&format!("127.0.0.1:{port_b}"))
        .with_tls_settings(TlsServerConfig::default().with_insecure(true));
    let client_to_a = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port_a}"))
        .with_tls_setting(TlsClientConfig::default().with_insecure(true))
        .with_connection_type(ConnType::Peer);

    let mut config_b = ServiceConfiguration::new();
    config_b.node_id = "late-join-b".to_string();
    let config_b = config_b
        .with_dataplane_server(vec![server_b])
        .with_dataplane_client(vec![client_to_a]);

    let svc_id_b = ID::new_with_str("slim/late-join-b").unwrap();
    let service_b = config_b.build_server(svc_id_b).unwrap();
    service_b.run().await.expect("failed to start node B");
    wait_for_server(&format!("127.0.0.1:{port_b}"), 40).await;

    // Wait for peer connection
    wait_for_peer_connections(&service_a, 1, Duration::from_secs(10)).await;
    wait_for_peer_connections(&service_b, 1, Duration::from_secs(10)).await;

    // Node B should have received the subscription from A via full sync
    let prefix = Name::from_strings(["org", "ns", "late-join"]);
    assert!(
        wait_for_subscription(&service_b, &prefix, Duration::from_secs(3)).await,
        "node B should have received subscription from A via full sync on connect"
    );

    service_a.shutdown().await.ok();
    service_b.shutdown().await.ok();
}
