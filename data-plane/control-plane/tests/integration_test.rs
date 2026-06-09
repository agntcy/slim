// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use rlimit::increase_nofile_limit;
use std::net::TcpListener;
use std::time::Duration;

use slim_auth::metadata::{MetadataMap, MetadataValue};
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
use slim_control_plane::config::{
    AdjacencyEntry, Config, DatabaseConfig, ReconcilerConfig, TopologyConfig,
};
use slim_control_plane::server::ControlPlane;
use slim_datapath::api::ProtoName as Name;
use slim_datapath::peer_discovery::{PeerConfig, PeerTopology, StaticPeerEntry};
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

fn grouped_node_id(group: &str, name: &str) -> String {
    format!("{group}/slim/{name}")
}

/// Start a node in a group with an external endpoint and optional peer configuration.
/// `peers_info` contains (node_id, endpoint) tuples for all nodes in the same group
/// (including self — self-entry is filtered out automatically by peer sync).
async fn start_grouped_node(
    name: &str,
    group: &str,
    southbound_port: u16,
    dp_port: u16,
    peers_info: &[(&str, u16)],
) -> Service {
    let dataplane_server = {
        let mut md = MetadataMap::new();
        md.insert(
            "external_endpoint",
            MetadataValue::String(format!("127.0.0.1:{dp_port}")),
        );
        let mut s = ServerConfig::with_endpoint(&format!("127.0.0.1:{dp_port}"))
            .with_tls_settings(TlsServerConfig::insecure());
        s.metadata = Some(md);
        s
    };

    let cp_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });

    let static_peers: Vec<StaticPeerEntry> = peers_info
        .iter()
        .map(|(peer_name, peer_port)| StaticPeerEntry {
            node_id: format!("slim/{peer_name}"),
            config: ClientConfig::with_endpoint(&format!("http://127.0.0.1:{peer_port}"))
                .with_tls_setting(TlsClientConfig::insecure()),
        })
        .collect();

    let peer_config = if static_peers.len() > 1 {
        Some(PeerConfig {
            peer_group: group.to_string(),
            topology: PeerTopology::FullMesh,
            static_peers,
            discovery: None,
        })
    } else {
        None
    };

    let mut service_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![dataplane_server])
        .with_controlplane_client(vec![cp_client]);
    service_config.group_name = Some(group.to_string());
    if let Some(pc) = peer_config {
        service_config = service_config.with_peers(pc);
    }

    let svc_id = ID::new_with_str(&node_id).unwrap();
    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

/// Start a node in a group (single node, no peers in same group).
async fn start_node_in_group(name: &str, group: &str, southbound_port: u16) -> Service {
    start_node_in_group_on_port(name, group, southbound_port, reserve_port()).await
}

/// Start a node in a group on a specific DP port (so a client app can connect).
async fn start_node_in_group_on_port(
    name: &str,
    group: &str,
    southbound_port: u16,
    dp_port: u16,
) -> Service {
    start_grouped_node(name, group, southbound_port, dp_port, &[(name, dp_port)]).await
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

/// Wait for an Applied link where the source node is in `group_a` and the
/// dest node is in `group_b` (or vice-versa). Group membership is inferred
/// from the node_id prefix (e.g. "group-a/slim/node" → group "group-a").
async fn wait_for_link_between_groups(
    client: &mut NbClient,
    group_a: &str,
    group_b: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let links = collect_links(client, "", "").await;

        let found = links.iter().any(|l| {
            if l.deleted || l.status != 2 {
                // status 2 = Applied
                return false;
            }
            let src_group = l.source_node_id.split('/').next().unwrap_or("");
            let dst_group = l.dest_node_id.split('/').next().unwrap_or("");
            (src_group == group_a && dst_group == group_b)
                || (src_group == group_b && dst_group == group_a)
        });

        if found {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            let link_info: Vec<_> = links
                .iter()
                .map(|l| {
                    format!(
                        "{}→{} [status={}]",
                        l.source_node_id, l.dest_node_id, l.status
                    )
                })
                .collect();
            panic!(
                "timeout waiting for Applied link between groups {group_a} and {group_b}. Links: [{}]",
                link_info.join(", ")
            );
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
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
            r.name.as_ref().unwrap()
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
        if expected_names
            .iter()
            .all(|n| subs.iter().any(|s| s.match_prefix(n)))
        {
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
        if absent_names
            .iter()
            .all(|n| !subs.iter().any(|s| s.match_prefix(n)))
        {
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

// All integration tests use 3 groups with full-mesh topology.
// The control-plane manages inter-group Remote links.
// Intra-group connectivity is handled by the data-plane peer sync.

/// Start control plane with a custom topology configuration.
async fn start_control_plane_with_topology(
    reconciler_config: ReconcilerConfig,
    topology: TopologyConfig,
) -> TestControlPlane {
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
        topology,
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

fn star_topology_config(hub_group: &str) -> TopologyConfig {
    TopologyConfig {
        links: vec![AdjacencyEntry {
            name: hub_group.to_string(),
            peers: vec!["*".to_string()],
        }],
    }
}

/// Connect a client app to a DP node and subscribe to a name.
/// Returns the Service handle (must be kept alive for the subscription to stay active).
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

/// Helper: create the standard 3-group test topology with 2 nodes per group.
/// Returns (control_plane, nb_client, nodes, node_ids, dp_ports).
/// Groups: "alpha", "beta", "gamma". Nodes: "{group}-1", "{group}-2" in each.
struct ThreeGroupSetup {
    cp: TestControlPlane,
    // nodes[0..1] = alpha, nodes[2..3] = beta, nodes[4..5] = gamma
    nodes: Vec<Service>,
    // dp_ports in same order as nodes
    dp_ports: Vec<u16>,
}

impl ThreeGroupSetup {
    fn node_id(&self, group: &str, idx: usize) -> String {
        grouped_node_id(group, &format!("{group}-{idx}"))
    }

    fn alpha_ids(&self) -> (String, String) {
        (self.node_id("alpha", 1), self.node_id("alpha", 2))
    }

    fn beta_ids(&self) -> (String, String) {
        (self.node_id("beta", 1), self.node_id("beta", 2))
    }

    fn gamma_ids(&self) -> (String, String) {
        (self.node_id("gamma", 1), self.node_id("gamma", 2))
    }

    async fn shutdown(self) {
        for node in self.nodes {
            node.shutdown().await.ok();
        }
        stop_control_plane(self.cp).await;
    }
}

async fn setup_three_groups(reconciler: ReconcilerConfig) -> ThreeGroupSetup {
    let cp = start_control_plane_with_topology(reconciler, TopologyConfig::default()).await;
    let sb = cp.southbound_port;

    // Reserve ports for all 6 nodes first
    let ports: Vec<u16> = (0..6).map(|_| reserve_port()).collect();

    let groups = ["alpha", "beta", "gamma"];
    let mut nodes = Vec::with_capacity(6);

    for (gi, group) in groups.iter().enumerate() {
        let p1 = ports[gi * 2];
        let p2 = ports[gi * 2 + 1];
        let name1 = format!("{group}-1");
        let name2 = format!("{group}-2");
        let peers_info: Vec<(&str, u16)> = vec![(&name1, p1), (&name2, p2)];

        // We need owned strings for the async calls
        let n1 = start_grouped_node(&name1, group, sb, p1, &peers_info).await;
        let n2 = start_grouped_node(&name2, group, sb, p2, &peers_info).await;
        nodes.push(n1);
        nodes.push(n2);
    }

    ThreeGroupSetup {
        cp,
        nodes,
        dp_ports: ports,
    }
}

/// Verify that inter-group links are created and claimed (Applied) for all
/// group pairs in the full-mesh topology.
#[tokio::test(flavor = "multi_thread")]
async fn test_inter_group_links_created_and_claimed() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let setup = setup_three_groups(test_reconciler_config()).await;
    let mut client = create_nb_client(setup.cp.northbound_port).await;

    let (a1, a2) = setup.alpha_ids();
    let (b1, b2) = setup.beta_ids();
    let (g1, g2) = setup.gamma_ids();

    // Wait for all 6 nodes to register
    wait_for_nodes_connected(
        &mut client,
        &[
            a1.as_str(),
            a2.as_str(),
            b1.as_str(),
            b2.as_str(),
            g1.as_str(),
            g2.as_str(),
        ],
        Duration::from_secs(15),
    )
    .await;

    // Wait for links between all group pairs to be Applied
    wait_for_link_between_groups(&mut client, "alpha", "beta", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "alpha", "gamma", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "beta", "gamma", Duration::from_secs(30)).await;

    // Verify link count: full mesh of 3 groups = 3 group-pairs.
    // Each pair has at least one link per direction, so at least 6 links.
    let links = collect_links(&mut client, "", "").await;
    let applied_links: Vec<_> = links
        .iter()
        .filter(|l| l.status == 2 && !l.deleted)
        .collect();
    assert!(
        applied_links.len() >= 3,
        "expected at least 3 Applied links for full mesh of 3 groups, got {}",
        applied_links.len()
    );

    print_state(&mut client, "inter_group_links: final").await;

    setup.shutdown().await;
}

/// Add routes via northbound API between nodes in different groups.
/// Verify routes are applied after links are established.
#[tokio::test(flavor = "multi_thread")]
async fn test_inter_group_route_applied() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let setup = setup_three_groups(test_reconciler_config()).await;
    let mut client = create_nb_client(setup.cp.northbound_port).await;

    let (a1, _a2) = setup.alpha_ids();
    let (b1, _b2) = setup.beta_ids();
    let (g1, _g2) = setup.gamma_ids();

    wait_for_nodes_connected(
        &mut client,
        &[a1.as_str(), b1.as_str(), g1.as_str()],
        Duration::from_secs(15),
    )
    .await;

    // Wait for inter-group links
    wait_for_link_between_groups(&mut client, "alpha", "beta", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "alpha", "gamma", Duration::from_secs(30)).await;

    // Add routes: alpha-1 → beta-1, alpha-1 → gamma-1
    add_route(&mut client, &a1, &b1, "org", "ns", "svc-ab").await;
    add_route(&mut client, &a1, &g1, "org", "ns", "svc-ag").await;

    // Wait for routes to be applied
    wait_for_route_applied(&mut client, &a1, &b1, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &a1, &g1, Duration::from_secs(30)).await;

    // Verify subscriptions on alpha-1's dataplane
    wait_for_node_subscriptions(
        &mut client,
        &a1,
        &[("org", "ns", "svc-ab"), ("org", "ns", "svc-ag")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "inter_group_route: final").await;

    setup.shutdown().await;
}

/// Client app subscribes on a node; route is propagated to nodes in other groups.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_propagation_across_groups() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let setup = setup_three_groups(test_reconciler_config()).await;
    let mut client = create_nb_client(setup.cp.northbound_port).await;

    let (a1, _) = setup.alpha_ids();
    let (b1, _) = setup.beta_ids();
    let (g1, _) = setup.gamma_ids();

    wait_for_nodes_connected(
        &mut client,
        &[a1.as_str(), b1.as_str(), g1.as_str()],
        Duration::from_secs(15),
    )
    .await;

    // Wait for links to be established
    wait_for_link_between_groups(&mut client, "alpha", "beta", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "alpha", "gamma", Duration::from_secs(30)).await;

    // Client app subscribes on alpha-1
    let alpha_dp_port = setup.dp_ports[0]; // alpha-1 port
    let app_svc = subscribe_via_link(alpha_dp_port, "org", "ns", "cross-group-svc").await;

    // Other groups should get routes to alpha-1
    wait_for_route_applied(&mut client, &b1, &a1, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &g1, &a1, Duration::from_secs(30)).await;

    // Verify subscriptions propagated to beta-1 and gamma-1
    wait_for_node_subscriptions(
        &mut client,
        &b1,
        &[("org", "ns", "cross-group-svc")],
        Duration::from_secs(15),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client,
        &g1,
        &[("org", "ns", "cross-group-svc")],
        Duration::from_secs(15),
    )
    .await;

    print_state(&mut client, "subscription_propagation: final").await;

    app_svc.shutdown().await.ok();
    setup.shutdown().await;
}

/// Verify bidirectional routes between groups: alpha-1 → beta-1 and beta-1 → alpha-1.
#[tokio::test(flavor = "multi_thread")]
async fn test_bidirectional_inter_group_routes() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let setup = setup_three_groups(test_reconciler_config()).await;
    let mut client = create_nb_client(setup.cp.northbound_port).await;

    let (a1, _) = setup.alpha_ids();
    let (b1, _) = setup.beta_ids();

    wait_for_nodes_connected(
        &mut client,
        &[a1.as_str(), b1.as_str()],
        Duration::from_secs(15),
    )
    .await;

    wait_for_link_between_groups(&mut client, "alpha", "beta", Duration::from_secs(30)).await;

    // Add bidirectional routes
    add_route(&mut client, &a1, &b1, "org", "ns", "svc-fwd").await;
    add_route(&mut client, &b1, &a1, "org", "ns", "svc-rev").await;

    wait_for_route_applied(&mut client, &a1, &b1, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &b1, &a1, Duration::from_secs(30)).await;

    // Verify subscriptions on both sides
    wait_for_node_subscriptions(
        &mut client,
        &a1,
        &[("org", "ns", "svc-fwd")],
        Duration::from_secs(10),
    )
    .await;
    wait_for_node_subscriptions(
        &mut client,
        &b1,
        &[("org", "ns", "svc-rev")],
        Duration::from_secs(10),
    )
    .await;

    print_state(&mut client, "bidirectional_routes: final").await;

    setup.shutdown().await;
}

/// Node crash and recovery: crash a node, verify link is re-established after restart.
#[tokio::test(flavor = "multi_thread")]
async fn test_node_crash_and_link_recovery() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let cp = start_control_plane_with_topology(test_reconciler_config(), TopologyConfig::default())
        .await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_node_in_group_on_port("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_node_in_group_on_port("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], Duration::from_secs(15)).await;

    wait_for_link_between_groups(&mut client, "group-a", "group-b", Duration::from_secs(30)).await;

    // Add a route
    add_route(&mut client, &id_a, &id_b, "org", "ns", "svc-crash").await;
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    print_state(&mut client, "Before node-b crash").await;

    // Crash node-b
    drop(node_b);
    tokio::time::sleep(Duration::from_millis(500)).await;

    print_state(&mut client, "After node-b crash").await;

    // Restart node-b on same port
    let node_b_new =
        start_node_in_group_on_port("node-b", "group-b", cp.southbound_port, b_port).await;

    wait_for_nodes_connected(&mut client, &[&id_b], Duration::from_secs(15)).await;

    // Link should be re-established
    wait_for_link_between_groups(&mut client, "group-a", "group-b", Duration::from_secs(30)).await;

    // Route should be re-applied
    wait_for_route_applied(&mut client, &id_a, &id_b, Duration::from_secs(30)).await;

    print_state(&mut client, "After node-b recovery").await;

    node_a.shutdown().await.ok();
    node_b_new.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Star topology: hub-and-spoke. Spokes only link to the hub, not to each other.
/// Routes between spokes transit through the hub.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "SPT routing requires full claim flow"]
async fn test_spt_route_via_hub() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    let cp = start_control_plane_with_topology(
        test_reconciler_config(),
        star_topology_config("platform"),
    )
    .await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_hub = grouped_node_id("platform", "hub");
    let id_spoke_a = grouped_node_id("customer-a", "spoke-a");
    let id_spoke_b = grouped_node_id("customer-b", "spoke-b");

    let spoke_a_dp_port = reserve_port();
    let hub = start_node_in_group("hub", "platform", cp.southbound_port).await;
    let spoke_a =
        start_node_in_group_on_port("spoke-a", "customer-a", cp.southbound_port, spoke_a_dp_port)
            .await;
    let spoke_b = start_node_in_group("spoke-b", "customer-b", cp.southbound_port).await;

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        Duration::from_secs(15),
    )
    .await;

    // Verify spoke-to-hub links exist, no spoke-to-spoke link.
    wait_for_link_between_groups(
        &mut client,
        "customer-a",
        "platform",
        Duration::from_secs(15),
    )
    .await;
    wait_for_link_between_groups(
        &mut client,
        "customer-b",
        "platform",
        Duration::from_secs(15),
    )
    .await;

    let all_links = collect_links(&mut client, "", "").await;
    let spoke_to_spoke = all_links.iter().any(|l| {
        if l.deleted {
            return false;
        }
        let src_g = l.source_node_id.split('/').next().unwrap_or("");
        let dst_g = l.dest_node_id.split('/').next().unwrap_or("");
        (src_g == "customer-a" && dst_g == "customer-b")
            || (src_g == "customer-b" && dst_g == "customer-a")
    });
    assert!(
        !spoke_to_spoke,
        "star topology should NOT create spoke-to-spoke links"
    );

    // A client app subscribes on spoke-a → CP creates wildcard *→spoke-a.
    // SPT rooted at spoke-a's group: spoke-b routes via hub.
    let app_svc = subscribe_via_link(spoke_a_dp_port, "org", "ns", "transit-svc").await;

    // spoke-b should get a route to spoke-a (via hub link).
    wait_for_route_applied(
        &mut client,
        &id_spoke_b,
        &id_spoke_a,
        Duration::from_secs(30),
    )
    .await;

    // hub should also have a route to spoke-a.
    wait_for_route_applied(&mut client, &id_hub, &id_spoke_a, Duration::from_secs(30)).await;

    // Verify the subscription is propagated to spoke-b.
    wait_for_node_subscriptions(
        &mut client,
        &id_spoke_b,
        &[("org", "ns", "transit-svc")],
        Duration::from_secs(15),
    )
    .await;

    print_state(&mut client, "spt_route_via_hub: final").await;

    app_svc.shutdown().await.ok();
    hub.shutdown().await.ok();
    spoke_a.shutdown().await.ok();
    spoke_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Full-mesh multicast: 3 nodes in different groups, all linked to each other.
/// Two nodes subscribe to the same name. Verify that:
/// (a) the first subscriber's SPT is used for all routing (single tree),
/// (b) the second subscriber gets a downward path from the root,
/// (c) no duplicate routes per (source, dest) pair exist.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "SPT routing requires full claim flow"]
async fn test_multicast_full_mesh_no_duplicate_routes() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();

    // Full mesh: empty topology = all groups link to all groups.
    let cp = start_control_plane_with_topology(test_reconciler_config(), TopologyConfig::default())
        .await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");
    let id_c = grouped_node_id("group-c", "node-c");

    let node_a_dp_port = reserve_port();
    let node_c_dp_port = reserve_port();
    let node_a =
        start_node_in_group_on_port("node-a", "group-a", cp.southbound_port, node_a_dp_port).await;
    let node_b = start_node_in_group("node-b", "group-b", cp.southbound_port).await;
    let node_c =
        start_node_in_group_on_port("node-c", "group-c", cp.southbound_port, node_c_dp_port).await;

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], Duration::from_secs(15)).await;

    // Wait for all inter-group links
    wait_for_link_between_groups(&mut client, "group-a", "group-b", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-c", Duration::from_secs(30)).await;
    wait_for_link_between_groups(&mut client, "group-b", "group-c", Duration::from_secs(30)).await;

    // First subscriber on node-a → becomes the SPT root.
    let app_a = subscribe_via_link(node_a_dp_port, "org", "ns", "mesh-multicast").await;

    // Wait for other nodes to get routes to node-a.
    wait_for_route_applied(&mut client, &id_b, &id_a, Duration::from_secs(30)).await;
    wait_for_route_applied(&mut client, &id_c, &id_a, Duration::from_secs(30)).await;

    // Second subscriber on node-c (same name) → downward path from root.
    let app_c = subscribe_via_link(node_c_dp_port, "org", "ns", "mesh-multicast").await;

    // node-a (root) should get a route to node-c (downward).
    wait_for_route_applied(&mut client, &id_a, &id_c, Duration::from_secs(30)).await;

    // Give the system a moment to settle.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify no duplicate routes for this name.
    let all_routes = collect_routes(&mut client, "", "").await;
    let multicast_routes: Vec<_> = all_routes
        .iter()
        .filter(|r| {
            r.name
                .as_ref()
                .is_some_and(|n| n.str_components().2 == "mesh-multicast")
                && r.source_node_id != "*"
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for r in &multicast_routes {
        let key = (r.source_node_id.clone(), r.dest_node_id.clone());
        assert!(
            seen.insert(key.clone()),
            "duplicate route found: {} → {}",
            key.0,
            key.1
        );
    }

    print_state(&mut client, "multicast_full_mesh: final").await;

    app_a.shutdown().await.ok();
    app_c.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    node_c.shutdown().await.ok();
    stop_control_plane(cp).await;
}
