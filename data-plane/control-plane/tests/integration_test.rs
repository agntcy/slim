// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use rlimit::increase_nofile_limit;
use std::net::TcpListener;
use std::time::Duration;
use uuid::Uuid;

use slim_auth::shared_secret::SharedSecret;
use slim_config::component::Component;
use slim_config::component::id::ID;
use slim_config::grpc::client::{ClientConfig, KeepaliveConfig};
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::provider::initialize_crypto_provider;
use slim_config::tls::server::TlsServerConfig;
use slim_control_plane::api::proto::controller::proto::v1::Route;
use slim_control_plane::api::proto::controlplane::proto::v1::control_plane_service_client::ControlPlaneServiceClient;
use slim_control_plane::api::proto::controlplane::proto::v1::{
    AddRouteRequest, LinkEntry, LinkListRequest, Node as CpNode, NodeEntry, NodeListRequest,
    RouteEntry, RouteListRequest,
};
use slim_control_plane::config::{Config, DatabaseConfig, ReconcilerConfig};
use slim_control_plane::server::ControlPlane;
use slim_datapath::api::ProtoName as Name;
use slim_service::{Service, ServiceConfiguration};
use slim_testing::utils::TEST_VALID_SECRET;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

// --- Helpers ---

fn raise_fd_limit() {
    static INIT_FD_LIMIT: std::sync::Once = std::sync::Once::new();
    INIT_FD_LIMIT.call_once(|| {
        let _ = increase_nofile_limit(4096).expect("unable to raise open file descriptor limit");
    });
}

fn reserve_port() -> u16 {
    raise_fd_limit();
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

struct TestControlPlane {
    northbound_port: u16,
    southbound_port: u16,
    cp: ControlPlane,
}

async fn start_control_plane(reconciler_config: ReconcilerConfig) -> TestControlPlane {
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

    let cp = ControlPlane::start(cfg)
        .await
        .expect("failed to start control plane");

    TestControlPlane {
        northbound_port,
        southbound_port,
        cp,
    }
}

async fn stop_control_plane(tcp: TestControlPlane) {
    tcp.cp.shutdown().await;
}

fn node_id(name: &str) -> String {
    format!("slim/{name}")
}

async fn start_node(name: &str, southbound_port: u16) -> Service {
    start_node_on_port(name, southbound_port, reserve_port()).await
}

async fn start_node_on_port(name: &str, southbound_port: u16, dp_port: u16) -> Service {
    let dataplane_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());

    let cp_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });

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

async fn collect_nodes(client: &mut NbClient) -> Vec<NodeEntry> {
    let mut stream = client
        .list_nodes(NodeListRequest {})
        .await
        .expect("list_nodes failed")
        .into_inner();
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry.expect("stream error"));
    }
    entries
}

async fn collect_routes(client: &mut NbClient, src: &str, dest: &str) -> Vec<RouteEntry> {
    let mut stream = client
        .list_routes(RouteListRequest {
            src_node_id: src.to_string(),
            dest_node_id: dest.to_string(),
        })
        .await
        .expect("list_routes failed")
        .into_inner();
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry.expect("stream error"));
    }
    entries
}

async fn collect_links(client: &mut NbClient, src: &str, dest: &str) -> Vec<LinkEntry> {
    let mut stream = client
        .list_links(LinkListRequest {
            src_node_id: src.to_string(),
            dest_node_id: dest.to_string(),
        })
        .await
        .expect("list_links failed")
        .into_inner();
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry.expect("stream error"));
    }
    entries
}

async fn wait_for_nodes_connected(client: &mut NbClient, node_ids: &[&str], timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let entries = collect_nodes(client).await;

        let connected_status = 1; // NodeStatus::Connected
        let all_connected = node_ids.iter().all(|id| {
            entries
                .iter()
                .any(|e| e.id == *id && e.status == connected_status)
        });

        if all_connected {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let statuses: Vec<_> = entries
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
        let routes = collect_routes(client, src, dest).await;

        let is_applied = routes.iter().any(|e| {
            e.source_node_id == src && e.dest_node_id == dest && e.status == applied_status
        });

        if is_applied {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let statuses: Vec<_> = routes
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
        let all_routes = collect_routes(client, "", "").await;

        let all_applied = routes.iter().all(|(src, dest)| {
            all_routes.iter().any(|e| {
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
                    !all_routes.iter().any(|e| {
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
        let links = collect_links(client, "", "").await;

        let found = links.iter().any(|l| {
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
            route: Some(Route {
                name: Some(Name::from_strings([component0, component1, component2])),
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
    let nodes = collect_nodes(client).await;
    println!("  Nodes ({}):", nodes.len());
    for n in &nodes {
        let status = match n.status {
            1 => "Connected",
            2 => "NotConnected",
            _ => "Unknown",
        };
        let endpoints: Vec<_> = n.connections.iter().map(|c| c.endpoint.as_str()).collect();
        println!("    - {} [{}] endpoints={:?}", n.id, status, endpoints);
    }

    // Routes
    let routes = collect_routes(client, "", "").await;
    println!("  Routes ({}):", routes.len());
    for r in &routes {
        let status = match r.status {
            1 => "Applied",
            2 => "Failed",
            4 => "Pending",
            _ => "Unknown",
        };
        let deleted = if r.status == 3 { " DELETED" } else { "" };
        println!(
            "    - {} -> {} [{status}{deleted}] sub={}",
            r.source_node_id,
            r.dest_node_id,
            r.name.as_ref().unwrap().to_string()
        );
    }

    // Links
    let links = collect_links(client, "", "").await;
    println!("  Links ({}):", links.len());
    for l in &links {
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

async fn get_node_subscriptions(client: &mut NbClient, node_id: &str) -> Vec<Name> {
    let resp = client
        .list_node_routes(CpNode {
            id: node_id.to_string(),
        })
        .await
        .expect("list_node_routes failed")
        .into_inner();
    resp.entries
        .iter()
        .map(|e| e.name.as_ref().unwrap().clone())
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
    let expected_names: Vec<Name> = expected
        .iter()
        .map(|(c0, c1, c2)| Name::from_strings([*c0, *c1, *c2]))
        .collect();
    loop {
        let subs = get_node_subscriptions(client, node_id).await;
        let all_present = expected_names.iter().all(|n| subs.contains(n));
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
    let absent_names: Vec<Name> = absent
        .iter()
        .map(|(c0, c1, c2)| Name::from_strings([*c0, *c1, *c2]))
        .collect();
    loop {
        let subs = get_node_subscriptions(client, node_id).await;
        let all_absent = absent_names.iter().all(|n| !subs.contains(n));
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
    println!("    Routes ({}):", subs.len());
    for n in &subs {
        println!("      - {n}");
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
    let all_links = collect_links(&mut client, "", "").await;

    let links_involving_a: Vec<_> = all_links
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
    let links_before = collect_links(&mut client, "", "").await;
    let link_before = links_before
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
    let links_after = collect_links(&mut client, "", "").await;
    let link_after = links_after
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
    let links_before = collect_links(&mut client, "", "").await;
    let link_before = links_before
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
        let links = collect_links(&mut client, "", "").await;

        let link_applied = links.iter().any(|l| l.link_id == link_id && l.status == 2);

        if link_applied {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            print_state(
                &mut client,
                "TIMEOUT - link not re-applied after source crash",
            )
            .await;
            panic!(
                "timeout waiting for link {link_id} to be re-applied after source node recovery"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Route should be re-applied after link recovery
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify link_id is preserved
    let links_after = collect_links(&mut client, "", "").await;
    let link_after = links_after
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
    const NUM_NODES: usize = 20;

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
    let nodes_list = collect_nodes(&mut client).await;
    let connected = nodes_list.iter().filter(|n| n.status == 1).count();
    println!(
        "\n=== Scale test summary ===\n  Nodes: {} total, {} connected",
        nodes_list.len(),
        connected
    );

    let routes = collect_routes(&mut client, "", "").await;

    let applied_count = routes.iter().filter(|e| e.status == 1).count();
    let pending_count = routes.iter().filter(|e| e.status == 4).count();
    let failed_count = routes.iter().filter(|e| e.status == 2).count();
    println!(
        "  Routes: {} total, {} applied, {} pending, {} failed",
        routes.len(),
        applied_count,
        pending_count,
        failed_count
    );

    let links = collect_links(&mut client, "", "").await;
    let links_applied = links.iter().filter(|l| l.status == 2).count();
    println!(
        "  Links: {} total, {} applied\n",
        links.len(),
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
    let n = Name::from_strings(["org", "ns", "svc-0"]);
    assert!(
        subs_0.contains(&n),
        "node-0 should have subscription org/ns/svc-0, got: {:?}",
        subs_0
    );
    // node-50 is source of route node-50->node-51 with sub org/ns/svc-50
    let mid = NUM_NODES / 2;
    let n_mid = Name::from_strings(["org", "ns", &format!("svc-{mid}")]);
    let subs_mid = get_node_subscriptions(&mut client, &node_id(&format!("node-{mid}"))).await;
    assert!(
        subs_mid.contains(&n_mid),
        "node-{mid} should have subscription org/ns/svc-{mid}, got: {:?}",
        subs_mid
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
    let links_before = collect_links(&mut client, "", "").await;
    let link_id = links_before
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
    let nodes = collect_nodes(&mut client).await;
    let node_b_exists = nodes.iter().any(|e| e.id == id_b);
    assert!(!node_b_exists, "node-b should be removed after deregister");

    // Active routes to node-b should be cleaned up
    let routes = collect_routes(&mut client, "", "").await;
    let active_route_to_b: Vec<_> = routes
        .iter()
        .filter(|r| r.dest_node_id == id_b && r.status != 3)
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
    let links = collect_links(&mut client, "", "").await;
    let active_links_with_b: Vec<_> = links
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
        let links = collect_links(&mut client, "", "").await;

        let link_applied = links.iter().any(|l| {
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
    let links_after = collect_links(&mut client, "", "").await;
    let link = links_after
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
    let cp_client_b =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
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
    let cp_client_a =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
            .with_tls_setting(TlsClientConfig::insecure());
    let mut static_client =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{node_b_dp_port}"))
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
    let links = collect_links(&mut client, "", "").await;

    let link_a_to_b = links
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

/// Tests bidirectional routes: A→B and B→A both get applied and subscriptions
/// propagate to both nodes' dataplanes.
#[tokio::test(flavor = "multi_thread")]
async fn test_bidirectional_routes() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc-ab").await;
    add_route(&mut client, &id_b, &id_a, "org", "ns", "svc-rev").await;

    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(30)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc-ab")],
        Duration::from_secs(10),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client,
        &id_b,
        &[("org", "ns", "svc-rev")],
        Duration::from_secs(10),
    )
    .await;

    // Cleanup
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Tests that when only the control plane restarts (nodes stay up), the DP
/// nodes reconnect automatically and routes are re-reconciled from persisted state.
#[tokio::test(flavor = "multi_thread")]
async fn test_cp_restart_nodes_reconnect() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();
    initialize_crypto_provider();

    let northbound_port = reserve_port();
    let southbound_port = reserve_port();

    let db_dir = tempfile::tempdir().expect("failed to create temp dir");
    let db_path = db_dir.path().join("cp.db").to_str().unwrap().to_string();

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    // ── Setup: Start CP + nodes, add route, verify applied ──────────────────

    let cfg = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::Sqlite {
            path: db_path.clone(),
        },
        reconciler: test_reconciler_config(),
        ..Default::default()
    };
    let cp = ControlPlane::start(cfg).await.expect("failed to start CP");

    let node_a = start_node("node-a", southbound_port).await;
    let node_b = start_node("node-b", southbound_port).await;

    let mut client = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "before CP restart").await;

    // ── Restart CP on same ports with same DB, nodes auto-reconnect ─────────

    cp.shutdown().await;
    drop(client);

    let cfg2 = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::Sqlite {
            path: db_path.clone(),
        },
        reconciler: test_reconciler_config(),
        ..Default::default()
    };
    let cp2 = ControlPlane::start(cfg2)
        .await
        .expect("failed to restart CP");

    let mut client2 = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client2, &[&id_a, &id_b], Duration::from_secs(30)).await;

    wait_for_link_between(&mut client2, &id_a, &id_b, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client2, &id_a, &id_b, Duration::from_secs(60)).await;

    wait_for_node_subscriptions(
        &mut client2,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(30),
    )
    .await;

    print_state(&mut client2, "after CP restart").await;

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    cp2.shutdown().await;
}

/// Tests that when nodes restart with different DP server ports, the control
/// plane detects the endpoint change and updates links accordingly.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "port-change support not yet implemented"]
async fn test_node_restart_with_port_change() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();
    initialize_crypto_provider();

    let northbound_port = reserve_port();
    let southbound_port = reserve_port();

    let db_dir = tempfile::tempdir().expect("failed to create temp dir");
    let db_path = db_dir.path().join("cp.db").to_str().unwrap().to_string();

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    // ── Phase 1: Start CP + nodes, add route, verify applied ────────────────

    let cfg = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::Sqlite {
            path: db_path.clone(),
        },
        reconciler: test_reconciler_config(),
        ..Default::default()
    };
    let cp = ControlPlane::start(cfg).await.expect("failed to start CP");

    let node_a = start_node("node-a", southbound_port).await;
    let node_b = start_node("node-b", southbound_port).await;

    let mut client = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc1").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "Phase 1: before node restart").await;

    // ── Phase 2: Restart nodes with new random ports ────────────────────────

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let node_a = start_node("node-a", southbound_port).await;
    let node_b = start_node("node-b", southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(30)).await;

    // The CP should detect the endpoint change, update the link, and
    // re-reconcile the route on the new connection.
    wait_for_link_between(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(60)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(30),
    )
    .await;

    print_state(&mut client, "Phase 2: after node restart (new ports)").await;
    print_node_state(&mut client, &id_a).await;

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    cp.shutdown().await;
}

// --- Subscription-driven route tests ---

/// Create a client-only SLIM node (no CP connection) that connects to a server
/// node's DP port, creates an app, and subscribes with forward_to through the
/// link. This emulates an application sending a subscribe message that reaches
/// the server node from a remote connection, triggering CP notification.
async fn subscribe_via_link(server_dp_port: u16, org: &str, ns: &str, component: &str) -> Service {
    let name = Name::from_strings([org, ns, component]);
    let client_cfg = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{server_dp_port}"))
        .with_tls_setting(TlsClientConfig::insecure());
    let endpoint = client_cfg.endpoint.clone();
    let svc_cfg = ServiceConfiguration::new().with_dataplane_client(vec![client_cfg]);
    let svc_id = ID::new_with_str(&format!("slim/app-{component}")).unwrap();
    let mut svc = svc_cfg.build_server(svc_id).unwrap();
    svc.start().await.unwrap();

    let conn_id = svc
        .get_connection_id(&endpoint)
        .expect("client connection not established");

    let (app, rx) = svc
        .create_app(
            &name,
            SharedSecret::new(component, TEST_VALID_SECRET).unwrap(),
            SharedSecret::new(component, TEST_VALID_SECRET).unwrap(),
        )
        .unwrap();
    app.subscribe(&name, Some(conn_id)).await.unwrap();
    std::mem::forget(app);
    std::mem::forget(rx);
    svc
}

/// Happy path: a client app connects to a DP node and subscribes through the
/// link. The CP receives the subscription and propagates the route to all other
/// nodes.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_route_propagation() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    // Start node-b on a known DP port so we can connect a client app to it.
    let node_b_dp_port = reserve_port();
    let node_b_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_b_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_b =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
            .with_tls_setting(TlsClientConfig::insecure())
            .with_keepalive(KeepaliveConfig {
                http2_keepalive: Duration::from_secs(1).into(),
                timeout: Duration::from_secs(1).into(),
                keep_alive_while_idle: true,
                ..Default::default()
            });
    let svc_b_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_b_dp_server])
        .with_controlplane_client(vec![cp_client_b]);
    let svc_b_id = ID::new_with_str("slim/node-b").unwrap();
    let mut node_b = svc_b_config.build_server(svc_b_id).unwrap();
    node_b.start().await.unwrap();

    let node_a = start_node("node-a", cp.southbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // A client app connects to node-b and subscribes through the link.
    // node-b receives the subscription from remote → reports to CP.
    // CP creates wildcard route (*→node-b) → expands to node-a→node-b.
    let app_svc = subscribe_via_link(node_b_dp_port, "org", "ns", "svc1").await;

    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify subscription pushed to node-a's dataplane
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc1")],
        Duration::from_secs(10),
    )
    .await;

    // Verify link exists from node-a to node-b
    wait_for_link_between(&mut client, &id_a, &id_b, Duration::from_secs(10)).await;

    print_state(&mut client, "subscription_route_propagation: final").await;

    app_svc.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Static link between two DP nodes: node-a has a static DP client connection
/// to node-b. A client app subscribes through node-a (which forwards to node-b
/// via the static link). After restarting node-a the route persists.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_with_static_link() {
    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    // Start node-b (server) on a known port.
    let node_b_dp_port = reserve_port();
    let node_b_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_b_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_b =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
            .with_tls_setting(TlsClientConfig::insecure())
            .with_keepalive(KeepaliveConfig {
                http2_keepalive: Duration::from_secs(1).into(),
                timeout: Duration::from_secs(1).into(),
                keep_alive_while_idle: true,
                ..Default::default()
            });
    let svc_b_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_b_dp_server])
        .with_controlplane_client(vec![cp_client_b]);
    let svc_b_id = ID::new_with_str("slim/node-b").unwrap();
    let mut node_b = svc_b_config.build_server(svc_b_id).unwrap();
    node_b.start().await.unwrap();

    // Start node-a with a static client connection to node-b.
    let static_link_id = Uuid::new_v4().to_string();
    let node_a_dp_port = reserve_port();
    let node_a_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_a_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_a =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{}", cp.southbound_port))
            .with_tls_setting(TlsClientConfig::insecure())
            .with_keepalive(KeepaliveConfig {
                http2_keepalive: Duration::from_secs(1).into(),
                timeout: Duration::from_secs(1).into(),
                keep_alive_while_idle: true,
                ..Default::default()
            });
    let mut static_client =
        ClientConfig::with_endpoint(&format!("http://127.0.0.1:{node_b_dp_port}"))
            .with_tls_setting(TlsClientConfig::insecure());
    static_client.link_id = static_link_id.clone();

    let svc_a_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_a_dp_server])
        .with_dataplane_client(vec![static_client.clone()])
        .with_controlplane_client(vec![cp_client_a]);
    let svc_a_id = ID::new_with_str("slim/node-a").unwrap();
    let mut node_a = svc_a_config.build_server(svc_a_id.clone()).unwrap();
    node_a.start().await.unwrap();

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // A client app connects to node-b and subscribes. node-b reports it to
    // the CP. The CP creates route *→node-b, expands to node-a→node-b, and
    // uses the static link from node-a to node-b.
    let app_svc = subscribe_via_link(node_b_dp_port, "org", "ns", "link-svc").await;

    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify the link uses the static link_id
    let links = collect_links(&mut client, "", "").await;
    let link_a_to_b = links
        .iter()
        .find(|l| l.source_node_id == id_a && l.dest_node_id == id_b)
        .expect("no link found from node-a to node-b");
    assert_eq!(
        link_a_to_b.link_id, static_link_id,
        "route should use static link"
    );

    // Verify subscription pushed to node-a's dataplane (via the route)
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "link-svc")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "subscription_with_static_link: before restart").await;

    // Restart node-a — route should persist in CP and be re-applied.
    app_svc.shutdown().await.ok();
    node_a.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut node_a = svc_a_config.build_server(svc_a_id).unwrap();
    node_a.start().await.unwrap();

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // Route should still be Applied after reconnection
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Subscription should still be on node-a's dataplane
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "link-svc")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "subscription_with_static_link: after restart").await;

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Nodes connect to CP, then CP goes away. Subscriptions arrive while CP is
/// unavailable and get buffered. CP restarts, nodes reconnect and forward the
/// pending subscriptions.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_before_cp_available() {
    initialize_crypto_provider();

    let southbound_port = reserve_port();
    let northbound_port = reserve_port();

    // Start the CP so nodes can connect initially.
    let cp = ControlPlane::start(Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::InMemory,
        reconciler: test_reconciler_config(),
        ..Default::default()
    })
    .await
    .expect("failed to start CP");

    let node_a_dp_port = reserve_port();
    let node_a_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_a_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_a = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });
    let svc_a_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_a_dp_server])
        .with_controlplane_client(vec![cp_client_a]);
    let mut node_a = svc_a_config
        .build_server(ID::new_with_str("slim/node-a").unwrap())
        .unwrap();
    node_a.start().await.unwrap();

    let node_b_dp_port = reserve_port();
    let node_b_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_b_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_b = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });
    let svc_b_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_b_dp_server])
        .with_controlplane_client(vec![cp_client_b]);
    let mut node_b = svc_b_config
        .build_server(ID::new_with_str("slim/node-b").unwrap())
        .unwrap();
    node_b.start().await.unwrap();

    let mut client = create_nb_client(northbound_port).await;
    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // Shut down the CP — nodes will lose connection and retry.
    cp.shutdown().await;
    drop(client);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Subscribe while CP is unavailable. The subscription messages buffer
    // in the controller's channel until the CP comes back.
    let app_a = subscribe_via_link(node_a_dp_port, "org", "ns", "svc-a").await;
    let app_b = subscribe_via_link(node_b_dp_port, "org", "ns", "svc-b").await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Restart the CP — nodes reconnect and deliver pending subscriptions.
    let cp = ControlPlane::start(Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::InMemory,
        reconciler: test_reconciler_config(),
        ..Default::default()
    })
    .await
    .expect("failed to restart CP");

    let mut client = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(30)).await;

    // node-a's subscription → route node-b→node-a
    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(30)).await;
    // node-b's subscription → route node-a→node-b
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    // Verify subscriptions on each other's dataplanes
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc-b")],
        Duration::from_secs(10),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client,
        &id_b,
        &[("org", "ns", "svc-a")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "subscription_before_cp_available: final").await;

    app_a.shutdown().await.ok();
    app_b.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    cp.shutdown().await;
}

/// CP restarts: subscriptions set before and after CP shutdown are all
/// reconciled once the CP comes back.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_survives_cp_restart() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();
    initialize_crypto_provider();

    let northbound_port = reserve_port();
    let southbound_port = reserve_port();
    let db_dir = tempfile::tempdir().expect("failed to create temp dir");
    let db_path = db_dir.path().join("cp.db").to_str().unwrap().to_string();

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    // Start node-a and node-b with known DP ports.
    let node_a_dp_port = reserve_port();
    let node_a_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_a_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_a = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });
    let svc_a_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_a_dp_server])
        .with_controlplane_client(vec![cp_client_a]);
    let svc_a_id = ID::new_with_str("slim/node-a").unwrap();
    let mut node_a = svc_a_config.build_server(svc_a_id).unwrap();

    let node_b_dp_port = reserve_port();
    let node_b_dp_server = ServerConfig::with_endpoint(&format!("127.0.0.1:{node_b_dp_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    let cp_client_b = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });
    let svc_b_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![node_b_dp_server])
        .with_controlplane_client(vec![cp_client_b]);
    let svc_b_id = ID::new_with_str("slim/node-b").unwrap();
    let mut node_b = svc_b_config.build_server(svc_b_id).unwrap();

    // Phase 1: Start CP + nodes, subscribe on both nodes, verify routes.
    let cfg = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::Sqlite {
            path: db_path.clone(),
        },
        reconciler: test_reconciler_config(),
        ..Default::default()
    };
    let cp = ControlPlane::start(cfg).await.expect("failed to start CP");

    node_a.start().await.unwrap();
    node_b.start().await.unwrap();

    let mut client = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // Subscribe on both nodes before CP goes down.
    let app_a = subscribe_via_link(node_a_dp_port, "org", "ns", "before-restart").await;
    let app_b = subscribe_via_link(node_b_dp_port, "org", "ns", "after-restart").await;

    // CP creates route node-b→node-a and node-a→node-b
    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_b,
        &[("org", "ns", "before-restart")],
        Duration::from_secs(10),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "after-restart")],
        Duration::from_secs(10),
    )
    .await;

    print_state(
        &mut client,
        "subscription_survives_cp_restart: before CP shutdown",
    )
    .await;

    // Phase 2: Shut down CP, verify nodes reconnect and routes persist.
    cp.shutdown().await;
    drop(client);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Phase 3: Restart CP with same DB. Nodes auto-reconnect.
    let cfg2 = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::Sqlite {
            path: db_path.clone(),
        },
        reconciler: test_reconciler_config(),
        ..Default::default()
    };
    let cp2 = ControlPlane::start(cfg2)
        .await
        .expect("failed to restart CP");

    let mut client2 = create_nb_client(northbound_port).await;
    wait_for_nodes_connected(&mut client2, &[&id_a, &id_b], Duration::from_secs(30)).await;

    // Both routes should re-apply after CP restart (persisted in DB + reconciled).
    wait_for_route_applied(&mut client2, &id_b, &id_a, Duration::from_secs(60)).await;
    wait_for_route_applied(&mut client2, &id_a, &id_b, Duration::from_secs(60)).await;

    // Verify subscriptions still on dataplanes after reconnection.
    wait_for_node_subscriptions(
        &mut client2,
        &id_b,
        &[("org", "ns", "before-restart")],
        Duration::from_secs(30),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client2,
        &id_a,
        &[("org", "ns", "after-restart")],
        Duration::from_secs(30),
    )
    .await;

    print_state(
        &mut client2,
        "subscription_survives_cp_restart: after CP restart",
    )
    .await;

    app_a.shutdown().await.ok();
    app_b.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    cp2.shutdown().await;
}

/// Rapid disconnect-reconnect: drop node-a's Service abruptly (no graceful
/// deregister), immediately restart node-a on the same DP port. Routes must
/// converge to Applied again. Validates node_locks serialization and
/// epoch-based stream replacement.
#[tokio::test(flavor = "multi_thread")]
async fn test_rapid_disconnect_reconnect() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    // Pin node-a's DP port so the restart reuses the same endpoint and the
    // CP's existing link record stays valid.
    let node_a_dp_port = reserve_port();
    let node_a = start_node_on_port("node-a", cp.southbound_port, node_a_dp_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc-rapid").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    print_state(&mut client, "rapid_disconnect_reconnect: before drop").await;

    // Drop node-a abruptly (no graceful deregister) and immediately restart it
    // on the same DP port. CP must replace the stream (epoch bump) and
    // re-reconcile routes.
    node_a.shutdown().await.ok();
    drop(node_a);

    let node_a2 = start_node_on_port("node-a", cp.southbound_port, node_a_dp_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(60)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_a,
        &[("org", "ns", "svc-rapid")],
        Duration::from_secs(30),
    )
    .await;

    print_state(&mut client, "rapid_disconnect_reconnect: after reconnect").await;

    node_a2.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Wildcard expansion on new node: install a wildcard route (*→A) via the
/// subscribe-via-link path before node-b exists. When node-b starts, the CP
/// must expand the wildcard into a concrete route node-b→node-a and reach
/// Applied. Validates ensure_routes_for_node's wildcard expansion.
#[tokio::test(flavor = "multi_thread")]
async fn test_wildcard_expansion_on_new_node() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    // Start node-a on a known DP port so a client app can subscribe through it
    // and trigger the CP to record a wildcard route (*→node-a).
    let node_a_dp_port = reserve_port();
    let node_a = start_node_on_port("node-a", cp.southbound_port, node_a_dp_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a], Duration::from_secs(10)).await;

    let app_svc = subscribe_via_link(node_a_dp_port, "org", "ns", "svc-wild").await;

    // Wait until the wildcard route exists in the CP (dest=node-a, any source).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let routes = collect_routes(&mut client, "", "").await;
        if routes.iter().any(|r| {
            r.dest_node_id == id_a && r.name.as_ref().unwrap().str_components().2 == "svc-wild"
        }) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for wildcard route to be installed");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    print_state(&mut client, "wildcard_expansion: after wildcard installed").await;

    // Now start node-b — the wildcard must expand into node-b→node-a.
    let node_b = start_node("node-b", cp.southbound_port).await;
    wait_for_nodes_connected(&mut client, &[&id_b], Duration::from_secs(10)).await;

    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(60)).await;

    wait_for_node_subscriptions(
        &mut client,
        &id_b,
        &[("org", "ns", "svc-wild")],
        Duration::from_secs(30),
    )
    .await;

    print_state(&mut client, "wildcard_expansion: after node-b joined").await;

    app_svc.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Bidirectional link reuse: add route A→B (which creates link A→B), then
/// add route B→A. The reverse route must reuse the existing link rather
/// than creating a new one, and both routes must reach Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_bidirectional_link_reuse() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let cp = start_control_plane(test_reconciler_config()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = node_id("node-a");
    let id_b = node_id("node-b");

    let node_a = start_node("node-a", cp.southbound_port).await;
    let node_b = start_node("node-b", cp.southbound_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(10)).await;

    // First route creates the link A→B.
    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc-fwd").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    let links_after_first = collect_links(&mut client, "", "").await;
    let active_links_first: Vec<_> = links_after_first.iter().filter(|l| !l.deleted).collect();
    assert_eq!(
        active_links_first.len(),
        1,
        "expected exactly one active link after first route, got: {:?}",
        active_links_first
            .iter()
            .map(|l| format!(
                "{}->{} link_id={}",
                l.source_node_id, l.dest_node_id, l.link_id
            ))
            .collect::<Vec<_>>()
    );
    let original_link_id = active_links_first[0].link_id.clone();

    // Reverse route should reuse the same link (bidirectional lookup).
    add_route(&mut client, &id_b, &id_a, "org", "ns", "svc-rev").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(30)).await;

    let links_after_second = collect_links(&mut client, "", "").await;
    let active_links_second: Vec<_> = links_after_second.iter().filter(|l| !l.deleted).collect();
    assert_eq!(
        active_links_second.len(),
        1,
        "expected the reverse route to reuse the existing link, got {} active links: {:?}",
        active_links_second.len(),
        active_links_second
            .iter()
            .map(|l| format!(
                "{}->{} link_id={}",
                l.source_node_id, l.dest_node_id, l.link_id
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        active_links_second[0].link_id, original_link_id,
        "reverse route should not have created a new link_id"
    );

    print_state(&mut client, "bidirectional_link_reuse: final").await;

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}
