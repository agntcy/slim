// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::TcpListener;
use std::time::Duration;
use uuid::Uuid;

use slim_config::component::Component;
use slim_config::component::id::ID;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::provider::initialize_crypto_provider;
use slim_config::tls::server::TlsServerConfig;
use slim_control_plane::api::proto::controller::proto::v1::Subscription;
use slim_control_plane::api::proto::controller::proto::v1::controller_service_server::ControllerServiceServer;
use slim_control_plane::api::proto::controlplane::proto::v1::control_plane_service_client::ControlPlaneServiceClient;
use slim_control_plane::api::proto::controlplane::proto::v1::control_plane_service_server::ControlPlaneServiceServer;
use slim_control_plane::api::proto::controlplane::proto::v1::{
    AddRouteRequest, LinkListRequest, Node as CpNode, NodeListRequest, RouteListRequest,
};
use slim_control_plane::config::{Config, DatabaseConfig, ReconcilerConfig};
use slim_control_plane::node_transport::DefaultNodeCommandHandler;
use slim_control_plane::services::northbound::NorthboundApiService;
use slim_control_plane::route_service::RouteService;
use slim_control_plane::services::southbound::SouthboundApiService;
use slim_service::{Service, ServiceConfiguration};

// --- Helpers ---

fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test port");
    let port = listener.local_addr().expect("failed to read port").port();
    drop(listener);
    port
}

fn test_reconciler_config() -> ReconcilerConfig {
    ReconcilerConfig {
        max_requeues: 10,
        base_retry_delay: Duration::from_millis(50).into(),
        reconcile_period: Duration::from_secs(0).into(),
        enable_orphan_detection: false,
        workers: 4,
    }
}

fn scale_reconciler_config() -> ReconcilerConfig {
    ReconcilerConfig {
        max_requeues: 15,
        base_retry_delay: Duration::from_millis(50).into(),
        reconcile_period: Duration::from_secs(0).into(),
        enable_orphan_detection: false,
        workers: 8,
    }
}

struct ControlPlaneInstance {
    northbound_port: u16,
    southbound_port: u16,
    route_service: RouteService,
    drain_tx: drain::Signal,
}

async fn start_control_plane(reconciler_config: ReconcilerConfig) -> ControlPlaneInstance {
    initialize_crypto_provider();

    let northbound_port = reserve_port();
    let southbound_port = reserve_port();

    let cfg = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::InMemory,
        reconciler: reconciler_config,
        ..Default::default()
    };

    let db = slim_control_plane::db::open(&cfg.database)
        .await
        .expect("failed to open DB");

    let cmd_handler = DefaultNodeCommandHandler::new();
    let route_service = RouteService::new(db.clone(), cmd_handler.clone(), cfg.reconciler);

    let nb_svc = NorthboundApiService::new(db.clone(), cmd_handler.clone(), route_service.clone());
    let sb_svc = SouthboundApiService::new(db, cmd_handler, route_service.clone());

    let (drain_tx, drain_rx) = drain::channel();

    cfg.northbound
        .run_server(&[ControlPlaneServiceServer::new(nb_svc)], drain_rx.clone())
        .await
        .expect("failed to start northbound server");

    cfg.southbound
        .run_server(&[ControllerServiceServer::new(sb_svc)], drain_rx)
        .await
        .expect("failed to start southbound server");

    tokio::time::sleep(Duration::from_millis(50)).await;

    ControlPlaneInstance {
        northbound_port,
        southbound_port,
        route_service,
        drain_tx,
    }
}

async fn stop_control_plane(cp: ControlPlaneInstance) {
    cp.route_service.shutdown().await;
    cp.drain_tx.drain().await;
}

fn node_id(name: &str) -> String {
    format!("slim/{name}")
}

async fn start_node(name: &str, southbound_port: u16) -> Service {
    let dp_port = reserve_port();

    let dataplane_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());

    let cp_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure());

    let service_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![dataplane_server])
        .with_controlplane_client(vec![cp_client]);

    let svc_id = ID::new_with_str(&format!("slim/{name}")).unwrap();
    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

type NbClient = ControlPlaneServiceClient<tonic::transport::Channel>;

async fn create_nb_client(northbound_port: u16) -> NbClient {
    ControlPlaneServiceClient::connect(format!("http://127.0.0.1:{northbound_port}"))
        .await
        .expect("failed to connect to northbound API")
}

async fn wait_for_nodes_connected(client: &mut NbClient, node_ids: &[&str], timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let resp = client
            .list_nodes(NodeListRequest {})
            .await
            .expect("list_nodes failed")
            .into_inner();

        let connected_status = 1; // NodeStatus::Connected
        let all_connected = node_ids.iter().all(|id| {
            resp.entries
                .iter()
                .any(|e| e.id == *id && e.status == connected_status)
        });

        if all_connected {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let statuses: Vec<_> = resp
                .entries
                .iter()
                .map(|e| format!("{}={}", e.id, e.status))
                .collect();
            panic!(
                "timeout waiting for nodes {:?} to connect. Current: [{}]",
                node_ids,
                statuses.join(", ")
            );
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_route_applied(client: &mut NbClient, src: &str, dest: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let applied_status = 1; // RouteStatus::Applied

    loop {
        let resp = client
            .list_routes(RouteListRequest {
                src_node_id: src.to_string(),
                dest_node_id: dest.to_string(),
            })
            .await
            .expect("list_routes failed")
            .into_inner();

        let is_applied = resp.routes.iter().any(|e| {
            e.source_node_id == src && e.dest_node_id == dest && e.status == applied_status
        });

        if is_applied {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let statuses: Vec<_> = resp
                .routes
                .iter()
                .map(|e| {
                    format!(
                        "{}->{}(status={})",
                        e.source_node_id, e.dest_node_id, e.status
                    )
                })
                .collect();
            panic!(
                "timeout waiting for route {src}->{dest} to be Applied. Current: [{}]",
                statuses.join(", ")
            );
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_all_routes_applied(
    client: &mut NbClient,
    routes: &[(&str, &str)],
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    let applied_status = 1;

    loop {
        let resp = client
            .list_routes(RouteListRequest {
                src_node_id: String::new(),
                dest_node_id: String::new(),
            })
            .await
            .expect("list_routes failed")
            .into_inner();

        let all_applied = routes.iter().all(|(src, dest)| {
            resp.routes.iter().any(|e| {
                e.source_node_id == *src && e.dest_node_id == *dest && e.status == applied_status
            })
        });

        if all_applied {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let pending: Vec<_> = routes
                .iter()
                .filter(|(src, dest)| {
                    !resp.routes.iter().any(|e| {
                        e.source_node_id == *src
                            && e.dest_node_id == *dest
                            && e.status == applied_status
                    })
                })
                .map(|(s, d)| format!("{s}->{d}"))
                .collect();
            panic!(
                "timeout waiting for all routes to be Applied. Pending: [{}]",
                pending.join(", ")
            );
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_link_between(
    client: &mut NbClient,
    node_a: &str,
    node_b: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let resp = client
            .list_links(LinkListRequest {
                src_node_id: String::new(),
                dest_node_id: String::new(),
            })
            .await
            .expect("list_links failed")
            .into_inner();

        let found = resp.links.iter().any(|l| {
            !l.deleted
                && ((l.source_node_id == node_a && l.dest_node_id == node_b)
                    || (l.source_node_id == node_b && l.dest_node_id == node_a))
        });

        if found {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for link between {node_a} and {node_b}");
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn add_route(
    client: &mut NbClient,
    src_node_id: &str,
    dest_node_id: &str,
    component0: &str,
    component1: &str,
    component2: &str,
) {
    let resp = client
        .add_route(AddRouteRequest {
            connection: None,
            node_id: src_node_id.to_string(),
            dest_node_id: dest_node_id.to_string(),
            subscription: Some(Subscription {
                component_0: component0.to_string(),
                component_1: component1.to_string(),
                component_2: component2.to_string(),
                id: None,
                connection_id: String::new(),
                node_id: None,
                link_id: None,
                direction: None,
            }),
        })
        .await
        .expect("add_route failed")
        .into_inner();

    assert!(resp.success, "add_route returned success=false");
}

async fn print_state(client: &mut NbClient, label: &str) {
    println!("\n=== {label} ===");

    // Nodes
    let nodes = client
        .list_nodes(NodeListRequest {})
        .await
        .expect("list_nodes failed")
        .into_inner();
    println!("  Nodes ({}):", nodes.entries.len());
    for n in &nodes.entries {
        let status = match n.status {
            1 => "Connected",
            2 => "NotConnected",
            _ => "Unknown",
        };
        let endpoints: Vec<_> = n.connections.iter().map(|c| c.endpoint.as_str()).collect();
        println!("    - {} [{}] endpoints={:?}", n.id, status, endpoints);
    }

    // Routes
    let routes = client
        .list_routes(RouteListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_routes failed")
        .into_inner();
    println!("  Routes ({}):", routes.routes.len());
    for r in &routes.routes {
        let status = match r.status {
            1 => "Applied",
            2 => "Failed",
            4 => "Pending",
            _ => "Unknown",
        };
        let deleted = if r.deleted { " DELETED" } else { "" };
        println!(
            "    - {} -> {} [{status}{deleted}] sub={}/{}/{}",
            r.source_node_id, r.dest_node_id, r.component_0, r.component_1, r.component_2
        );
    }

    // Links
    let links = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    println!("  Links ({}):", links.links.len());
    for l in &links.links {
        let status = match l.status {
            1 => "Pending",
            2 => "Applied",
            3 => "Failed",
            _ => "Unknown",
        };
        let deleted = if l.deleted { " DELETED" } else { "" };
        println!(
            "    - {} -> {} [{status}{deleted}] endpoint={} link_id={}",
            l.source_node_id, l.dest_node_id, l.dest_endpoint, l.link_id
        );
    }
    println!();
}

async fn get_node_subscriptions(
    client: &mut NbClient,
    node_id: &str,
) -> Vec<(String, String, String)> {
    let resp = client
        .list_subscriptions(CpNode {
            id: node_id.to_string(),
        })
        .await
        .expect("list_subscriptions failed")
        .into_inner();
    resp.entries
        .iter()
        .map(|e| {
            (
                e.component_0.clone(),
                e.component_1.clone(),
                e.component_2.clone(),
            )
        })
        .collect()
}

async fn get_node_connections(client: &mut NbClient, node_id: &str) -> Vec<String> {
    let resp = client
        .list_connections(CpNode {
            id: node_id.to_string(),
        })
        .await
        .expect("list_connections failed")
        .into_inner();
    resp.entries
        .iter()
        .filter_map(|e| e.link_id.clone())
        .collect()
}

async fn wait_for_node_subscriptions(
    client: &mut NbClient,
    node_id: &str,
    expected: &[(&str, &str, &str)],
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let subs = get_node_subscriptions(client, node_id).await;
        let all_present = expected
            .iter()
            .all(|(c0, c1, c2)| subs.contains(&(c0.to_string(), c1.to_string(), c2.to_string())));
        if all_present {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for subscriptions {:?} on node {node_id}. Current: {:?}",
                expected, subs
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_node_subscriptions_absent(
    client: &mut NbClient,
    node_id: &str,
    absent: &[(&str, &str, &str)],
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let subs = get_node_subscriptions(client, node_id).await;
        let all_absent = absent
            .iter()
            .all(|(c0, c1, c2)| !subs.contains(&(c0.to_string(), c1.to_string(), c2.to_string())));
        if all_absent {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for subscriptions {:?} to be absent on node {node_id}. Current: {:?}",
                absent, subs
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn print_node_state(client: &mut NbClient, node_id: &str) {
    let subs = get_node_subscriptions(client, node_id).await;
    let resp = client
        .list_connections(CpNode {
            id: node_id.to_string(),
        })
        .await
        .expect("list_connections failed")
        .into_inner();
    println!("  Node {node_id} dataplane state:");
    println!("    Subscriptions ({}):", subs.len());
    for (c0, c1, c2) in &subs {
        println!("      - {c0}/{c1}/{c2}");
    }
    println!("    Connections ({}):", resp.entries.len());
    for entry in &resp.entries {
        let dir = if entry.direction == 0 { "out" } else { "in" };
        let lid = entry.link_id.as_deref().unwrap_or("<none>");
        println!("      - link_id={lid} [{dir}]");
    }
}

// --- Tests ---

#[tokio::test(flavor = "multi_thread")]
async fn test_basic_subscription_forwarding() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;
    let node_c = start_node("node-c", cp.southbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");
    let id_c = node_id("node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], Duration::from_secs(10)).await;

    print_state(&mut client, "After nodes connected").await;

    // Add routes
    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    add_route(&mut client, &id_a, &id_c, "org", "ns", "svc2").await;

    // Wait for routes to be applied
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_a, &id_c, Duration::from_secs(30)).await;

    print_state(&mut client, "After routes applied").await;

    // Verify links exist between the nodes (direction may vary)
    let all_links = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();

    let links_involving_a: Vec<_> = all_links
        .links
        .iter()
        .filter(|l| l.source_node_id == id_a || l.dest_node_id == id_a)
        .collect();

    assert!(
        links_involving_a.len() >= 2,
        "expected at least 2 links involving node-a, got {}",
        links_involving_a.len(),
    );

    // Verify subscriptions are actually applied on node-a's dataplane
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1"), ("org", "ns", "svc2")],
        Duration::from_secs(10),
    )
    .await;
    print_node_state(&mut client, &id_a).await;

    // Verify node-a has connections (links) established
    let conns_a = get_node_connections(&mut client, &id_a).await;
    assert!(!conns_a.is_empty(), "node-a should have active connections");

    // Cleanup
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    node_c.shutdown().await.ok();
    stop_control_plane(cp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dest_node_crash_and_recovery() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;
    wait_for_link_between(&mut client, &id_a, &id_b, Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Capture link_id before crash
    let links_before = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link_before = links_before
        .links
        .iter()
        .find(|l| {
            (l.source_node_id == id_a && l.dest_node_id == id_b)
                || (l.source_node_id == id_b && l.dest_node_id == id_a)
        })
        .expect("expected a link between node-a and node-b");
    let link_id = link_before.link_id.clone();
    let link_source = link_before.source_node_id.clone();

    print_state(&mut client, "Before node-b crash").await;

    // Crash destination node B (no graceful deregister)
    drop(node_b);
    tokio::time::sleep(Duration::from_millis(500)).await;

    print_state(&mut client, "After node-b crash").await;

    // Restart node B
    let node_b_new = start_node("node-b", cp.southbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_b], Duration::from_secs(10)).await;

    // Route should be re-reconciled
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify link_id is preserved
    let links_after = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link_after = links_after
        .links
        .iter()
        .find(|l| l.link_id == link_id)
        .expect("link should exist with same link_id after recovery");
    assert_eq!(link_after.status, 2, "link should be Applied");

    // Verify connection is on link-source's dataplane
    let conns = get_node_connections(&mut client, &link_source).await;
    assert!(
        conns.contains(&link_id),
        "link source should have connection with link_id={link_id}, got: {conns:?}",
    );

    // Verify subscription is restored on source node's dataplane
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    // Cleanup
    node_a.shutdown().await.ok();
    node_b_new.shutdown().await.ok();
    stop_control_plane(cp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_source_node_crash_and_recovery() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;
    wait_for_link_between(&mut client, &id_a, &id_b, Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Capture link_id before crash
    let links_before = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link_before = links_before
        .links
        .iter()
        .find(|l| {
            (l.source_node_id == id_a && l.dest_node_id == id_b)
                || (l.source_node_id == id_b && l.dest_node_id == id_a)
        })
        .expect("expected a link between node-a and node-b");
    let link_id = link_before.link_id.clone();
    let link_source = link_before.source_node_id.clone();

    print_state(&mut client, "Before node-a crash").await;

    // Crash source node A (no graceful deregister)
    drop(node_a);
    tokio::time::sleep(Duration::from_millis(500)).await;

    print_state(&mut client, "After node-a crash").await;

    // Restart node A
    let node_a_new = start_node("node-a", cp.southbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a], Duration::from_secs(10)).await;

    // Wait for the link to be re-established (it was set to Pending during re-registration)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let links = client
            .list_links(LinkListRequest {
                src_node_id: String::new(),
                dest_node_id: String::new(),
            })
            .await
            .expect("list_links failed")
            .into_inner();

        let link_applied = links
            .links
            .iter()
            .any(|l| l.link_id == link_id && l.status == 2);

        if link_applied {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            print_state(&mut client, "TIMEOUT - link not re-applied after source crash").await;
            panic!("timeout waiting for link {link_id} to be re-applied after source node recovery");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Route should be re-applied after link recovery
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify link_id is preserved
    let links_after = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link_after = links_after
        .links
        .iter()
        .find(|l| l.link_id == link_id)
        .expect("link should exist with same link_id after recovery");
    assert_eq!(link_after.status, 2, "link should be Applied");

    // Verify connection is on link-source's dataplane
    let conns = get_node_connections(&mut client, &link_source).await;
    assert!(
        conns.contains(&link_id),
        "link source should have connection with link_id={link_id}, got: {conns:?}",
    );

    // NOTE: Subscription recovery after source crash is blocked by a DP-level
    // issue: when node-b reconnects to node-a's new address, node-a does not
    // register the incoming connection with the link_id. The reconciler cannot
    // push the subscription until this is fixed in the Service/DP layer.

    // Cleanup
    node_a_new.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scale_many_nodes() {
    const NUM_NODES: usize = 100;

    let cp = start_control_plane(scale_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    // Start all nodes concurrently
    let southbound_port = cp.southbound_port;
    let mut nodes = Vec::with_capacity(NUM_NODES);
    let mut join_set = tokio::task::JoinSet::new();

    for i in 0..NUM_NODES {
        let name = format!("node-{i}");
        join_set.spawn(async move { start_node(&name, southbound_port).await });
    }

    while let Some(result) = join_set.join_next().await {
        nodes.push(result.expect("node start task panicked"));
    }

    // Wait for all nodes to be connected
    let node_ids: Vec<String> = (0..NUM_NODES)
        .map(|i| node_id(&format!("node-{i}")))
        .collect();
    let node_id_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();

    wait_for_nodes_connected(&mut client, &node_id_refs, Duration::from_secs(30)).await;

    // Add routes forming a chain: node-0->node-1, node-1->node-2, ..., node-98->node-99
    for i in 0..(NUM_NODES - 1) {
        add_route(
            &mut client,
            &node_id(&format!("node-{i}")),
            &node_id(&format!("node-{}", i + 1)),
            "org",
            "ns",
            &format!("svc-{i}"),
        )
        .await;
    }

    // Wait for all routes to become Applied
    let route_pairs: Vec<(String, String)> = (0..(NUM_NODES - 1))
        .map(|i| {
            (
                node_id(&format!("node-{i}")),
                node_id(&format!("node-{}", i + 1)),
            )
        })
        .collect();
    let route_refs: Vec<(&str, &str)> = route_pairs
        .iter()
        .map(|(s, d)| (s.as_str(), d.as_str()))
        .collect();

    wait_for_all_routes_applied(&mut client, &route_refs, Duration::from_secs(120)).await;

    // Print summary
    let nodes_resp = client
        .list_nodes(NodeListRequest {})
        .await
        .expect("list_nodes failed")
        .into_inner();
    let connected = nodes_resp.entries.iter().filter(|n| n.status == 1).count();
    println!(
        "\n=== Scale test summary ===\n  Nodes: {} total, {} connected",
        nodes_resp.entries.len(),
        connected
    );

    let routes = client
        .list_routes(RouteListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_routes failed")
        .into_inner();

    let applied_count = routes.routes.iter().filter(|e| e.status == 1).count();
    let pending_count = routes.routes.iter().filter(|e| e.status == 4).count();
    let failed_count = routes.routes.iter().filter(|e| e.status == 2).count();
    println!(
        "  Routes: {} total, {} applied, {} pending, {} failed",
        routes.routes.len(),
        applied_count,
        pending_count,
        failed_count
    );

    let links = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let links_applied = links.links.iter().filter(|l| l.status == 2).count();
    println!(
        "  Links: {} total, {} applied\n",
        links.links.len(),
        links_applied
    );

    assert!(
        applied_count >= NUM_NODES - 1,
        "expected at least {} applied routes, got {applied_count}",
        NUM_NODES - 1
    );

    // Spot-check: verify subscriptions are actually on the nodes' dataplanes
    // node-0 is source of route node-0->node-1 with sub org/ns/svc-0
    let subs_0 = get_node_subscriptions(&mut client, &node_id("node-0")).await;
    assert!(
        subs_0.contains(&("org".to_string(), "ns".to_string(), "svc-0".to_string())),
        "node-0 should have subscription org/ns/svc-0, got: {:?}",
        subs_0
    );
    // node-50 is source of route node-50->node-51 with sub org/ns/svc-50
    let subs_50 = get_node_subscriptions(&mut client, &node_id("node-50")).await;
    assert!(
        subs_50.contains(&("org".to_string(), "ns".to_string(), "svc-50".to_string())),
        "node-50 should have subscription org/ns/svc-50, got: {:?}",
        subs_50
    );

    // Cleanup
    for node in nodes {
        node.shutdown().await.ok();
    }
    stop_control_plane(cp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_node_deregister_and_reregister() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;
    wait_for_link_between(&mut client, &id_a, &id_b, Duration::from_secs(10)).await;

    // Add route A->B and wait for it to be applied
    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify subscription is on node-a
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    // Capture the link_id before deregister — it should be preserved
    let links_before = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link_id = links_before
        .links
        .iter()
        .find(|l| {
            (l.source_node_id == id_a && l.dest_node_id == id_b)
                || (l.source_node_id == id_b && l.dest_node_id == id_a)
        })
        .expect("expected a link between node-a and node-b")
        .link_id
        .clone();

    print_state(&mut client, "Before node-b deregister").await;

    // Gracefully deregister node-b and shut down
    node_b.deregister().await.expect("deregister failed");
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;

    print_state(&mut client, "After node-b deregister").await;

    // --- Verify cleanup ---

    // Node-b should be gone from the node list
    let nodes = client
        .list_nodes(NodeListRequest {})
        .await
        .expect("list_nodes failed")
        .into_inner();
    let node_b_exists = nodes.entries.iter().any(|e| e.id == id_b);
    assert!(!node_b_exists, "node-b should be removed after deregister");

    // Active routes to node-b should be cleaned up
    let routes = client
        .list_routes(RouteListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_routes failed")
        .into_inner();
    let active_route_to_b: Vec<_> = routes
        .routes
        .iter()
        .filter(|r| r.dest_node_id == id_b && !r.deleted)
        .collect();
    assert!(
        active_route_to_b.is_empty(),
        "active routes to node-b should be cleaned up after deregister, got: {:?}",
        active_route_to_b
            .iter()
            .map(|r| format!(
                "{}->{} (status={})",
                r.source_node_id, r.dest_node_id, r.status
            ))
            .collect::<Vec<_>>()
    );

    // Links involving node-b should be soft-deleted
    let links = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let active_links_with_b: Vec<_> = links
        .links
        .iter()
        .filter(|l| (l.source_node_id == id_b || l.dest_node_id == id_b) && !l.deleted)
        .collect();
    assert!(
        active_links_with_b.is_empty(),
        "active links involving node-b should be gone after deregister, got: {:?}",
        active_links_with_b
            .iter()
            .map(|l| format!("{}->{}", l.source_node_id, l.dest_node_id))
            .collect::<Vec<_>>()
    );

    // Subscription should be removed from node-a's dataplane
    wait_for_node_subscriptions_absent(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    // --- Re-register node-b and verify restoration ---

    let node_b_new = start_node("node-b", cp.southbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_b], Duration::from_secs(10)).await;

    // Link should be restored with same link_id and reach Applied status
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let links = client
            .list_links(LinkListRequest {
                src_node_id: String::new(),
                dest_node_id: String::new(),
            })
            .await
            .expect("list_links failed")
            .into_inner();

        let link_applied = links.links.iter().any(|l| {
            l.link_id == link_id && l.status == 2 // Applied
        });

        if link_applied {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            print_state(&mut client, "TIMEOUT - link not reapplied").await;
            panic!("timeout waiting for link {link_id} to be reapplied after re-register");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Route should be automatically restored
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    print_state(
        &mut client,
        "After node-b re-register (link + route restored)",
    )
    .await;

    // Verify the link_id is preserved
    let links_after = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let link = links_after
        .links
        .iter()
        .find(|l| l.link_id == link_id)
        .expect("link should exist with the same link_id after re-register");
    assert_eq!(link.status, 2, "link should be Applied");

    // Verify connection is on the link source node's dataplane
    let link_source = &link.source_node_id;
    let conns = get_node_connections(&mut client, link_source).await;
    assert!(
        conns.contains(&link_id),
        "link source {} should have connection with link_id={link_id}, got: {:?}",
        link_source,
        conns
    );

    // Verify the subscription is re-pushed to node-a
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "Final state").await;
    print_node_state(&mut client, &id_a).await;

    // Cleanup
    node_a.shutdown().await.ok();
    node_b_new.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Tests that the CP adopts a static connection (configured in DP's client config)
/// instead of generating its own connection config for the link.
#[tokio::test(flavor = "multi_thread")]
async fn test_static_connection_adoption() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    // Start node-b first so we know its dataplane port.
    let node_b_dp_port = reserve_port();
    let node_b_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_b_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_b = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
        .with_tls_setting(TlsClientConfig::insecure());
    let svc_b_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_b_dp_server])
        .with_controlplane_client(vec![cp_client_b]);
    let svc_b_id = ID::new_with_str("slim/node-b").unwrap();
    let mut node_b = svc_b_config.build_server(svc_b_id).unwrap();
    node_b.start().await.unwrap();

    // Start node-a with a static client connection to node-b's dataplane.
    let static_link_id = Uuid::new_v4().to_string();
    let node_a_dp_port = reserve_port();
    let node_a_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_a_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_a = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
        .with_tls_setting(TlsClientConfig::insecure());
    let mut static_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{node_b_dp_port}"))
        .with_tls_setting(TlsClientConfig::insecure());
    static_client.link_id = static_link_id.clone();

    let svc_a_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_a_dp_server])
        .with_dataplane_client(vec![static_client])
        .with_controlplane_client(vec![cp_client_a]);
    let svc_a_id = ID::new_with_str("slim/node-a").unwrap();
    let mut node_a = svc_a_config.build_server(svc_a_id).unwrap();
    node_a.start().await.unwrap();

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // Add a route from node-a to node-b.
    add_route(&mut client, &id_a, &id_b, "org", "ns", "static-svc").await;

    // Wait for the route to be applied.
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify the link uses the static connection's link_id.
    let links = client
        .list_links(LinkListRequest {
            src_node_id: String::new(),
            dest_node_id: String::new(),
        })
        .await
        .expect("list_links failed")
        .into_inner();

    let link_a_to_b = links
        .links
        .iter()
        .find(|l| l.source_node_id == id_a && l.dest_node_id == id_b)
        .expect("no link found from node-a to node-b");

    assert_eq!(
        link_a_to_b.link_id, static_link_id,
        "link should use the static connection's link_id, got {} instead of {}",
        link_a_to_b.link_id, static_link_id
    );

    // Verify the link is Applied (not Pending — it was already established).
    let applied_status = 2; // LinkStatus::Applied
    assert_eq!(
        link_a_to_b.status, applied_status,
        "link should be Applied (status=2), got status={}",
        link_a_to_b.status
    );

    print_state(&mut client, "After static connection adoption").await;

    // Cleanup
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}
