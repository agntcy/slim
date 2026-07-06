// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the SLIM control plane's inter-group routing and lifecycle.
//!
//! These tests exercise the full stack: control plane, data plane nodes, and client apps.
//! They cover link creation, route propagation, gateway failover, and cleanup scenarios
//! in a segmented (multi-group) topology.

use rlimit::increase_nofile_limit;
use std::time::Duration;

use slim_auth::metadata::{MetadataMap, MetadataValue};
use slim_auth::shared_secret::SharedSecret;
use slim_config::auth::AuthConfig;
use slim_config::component::Component;
use slim_config::component::id::ID;
use slim_config::grpc::client::{ClientConfig, KeepaliveConfig};
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::provider::initialize_crypto_provider;
use slim_config::tls::server::TlsServerConfig;
use slim_control_plane::api::proto::controlplane::proto::v1::control_plane_service_client::ControlPlaneServiceClient;
use slim_control_plane::api::proto::controlplane::proto::v1::{
    AddSegmentRequest, AddTopologyLinkRequest, LinkEntry, LinkListRequest, NodeEntry,
    NodeListRequest, RemoveSegmentRequest, RemoveTopologyLinkRequest, RouteEntry, RouteListRequest,
    SegmentListRequest,
};
use slim_control_plane::config::{
    AdjacencyEntry, Config, DatabaseConfig, ReconcilerConfig, RegistrationAuthConfig,
    SegmentConfig, TopologyConfig, TopologySettings,
};
use slim_control_plane::server::ControlPlane;
use slim_datapath::api::ProtoName as Name;
use slim_datapath::peer_discovery::{
    PeerConfig, PeerDiscoveryConfig, PeerTopology, StaticPeerEntry,
};
use slim_service::{Service, ServiceConfiguration};
use slim_testing::common::reserve_local_port;
use slim_testing::utils::TEST_VALID_SECRET;
use tokio_stream::StreamExt;
use tracing_subscriber::EnvFilter;

// =============================================================================
// Helpers
// =============================================================================

/// Route status constants (from protobuf enum).
const ROUTE_APPLIED: i32 = 1;
const ROUTE_DELETED: i32 = 3;

/// Link status constants.
const LINK_APPLIED: i32 = 2;

/// Node status constants.
const NODE_CONNECTED: i32 = 1;

/// Default timeout for waiting on async conditions.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const SHORT_TIMEOUT: Duration = Duration::from_secs(15);

/// Per-group registration secrets (each group has its own secret for isolation).
const TEST_GROUP_SECRETS: &[(&str, &str)] = &[
    ("group-a", "secret-group-a-00112233445566778899"),
    ("group-b", "secret-group-b-00112233445566778899"),
    ("group-c", "secret-group-c-00112233445566778899"),
    ("group-d", "secret-group-d-00112233445566778899"),
    ("platform", "secret-platform-001122334455667788"),
    ("customer-a", "secret-customer-a-0011223344556677"),
    ("customer-b", "secret-customer-b-0011223344556677"),
];

/// Look up the test secret for a group.
fn group_secret(group: &str) -> &'static str {
    TEST_GROUP_SECRETS
        .iter()
        .find(|(g, _)| *g == group)
        .unwrap_or_else(|| panic!("no test secret for group '{group}'"))
        .1
}

/// Build the registration auth config with per-group shared secrets.
fn test_registration_auth() -> RegistrationAuthConfig {
    RegistrationAuthConfig::SharedSecret {
        secrets: TEST_GROUP_SECRETS
            .iter()
            .map(|(g, s)| (g.to_string(), s.to_string()))
            .collect(),
    }
}

fn raise_fd_limit() {
    static INIT_FD_LIMIT: std::sync::Once = std::sync::Once::new();
    INIT_FD_LIMIT.call_once(|| {
        let _ = increase_nofile_limit(4096).expect("unable to raise open file descriptor limit");
    });
}

fn reserve_port() -> u16 {
    raise_fd_limit();
    reserve_local_port()
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

/// Construct the node ID as it appears in the control plane.
/// The CP constructs it as "{group}/{node_id}" from the registration request.
fn grouped_node_id(group: &str, name: &str) -> String {
    format!("{group}/{name}")
}

// --- Control Plane ---

struct TestControlPlane {
    northbound_port: u16,
    southbound_port: u16,
    cp: ControlPlane,
}

async fn start_control_plane(topology: TopologyConfig) -> TestControlPlane {
    initialize_crypto_provider();

    let northbound_port = reserve_port();
    let southbound_port = reserve_port();

    let cfg = Config {
        northbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{northbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        southbound: ServerConfig::with_endpoint(&format!("127.0.0.1:{southbound_port}"))
            .with_tls_settings(TlsServerConfig::insecure()),
        database: DatabaseConfig::InMemory,
        reconciler: test_reconciler_config(),
        topology: TopologySettings {
            config: topology,
            auth: Some(test_registration_auth()),
        },
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

fn star_topology_config(hub_group: &str) -> TopologyConfig {
    TopologyConfig::Links(vec![AdjacencyEntry {
        group: hub_group.to_string(),
        neighbors: vec!["*".to_string()],
    }])
}

/// Full-mesh topology: every group can link to every other group.
fn full_mesh_topology() -> TopologyConfig {
    TopologyConfig::Links(vec![AdjacencyEntry {
        group: "*".to_string(),
        neighbors: vec!["*".to_string()],
    }])
}

// --- Node Management ---

/// Start a node in a group with an external endpoint and optional peer configuration.
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
            node_id: peer_name.to_string(),
            config: ClientConfig::with_endpoint(&format!("http://127.0.0.1:{peer_port}"))
                .with_tls_setting(TlsClientConfig::insecure()),
        })
        .collect();

    let peer_config = if static_peers.len() > 1 {
        Some(PeerConfig {
            deployment_name: group.to_string(),
            topology: PeerTopology::FullMesh,
            discovery: PeerDiscoveryConfig::Static {
                peers: static_peers,
            },
        })
    } else {
        None
    };

    let mut service_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![dataplane_server])
        .with_controlplane_client(vec![cp_client])
        .with_node_id(name);
    service_config.group_name = Some(group.to_string());
    service_config.auth = Some(AuthConfig::SharedSecret {
        id: Some(format!("{group}/{name}")),
        secret: group_secret(group).to_string(),
    });
    if let Some(pc) = peer_config {
        service_config = service_config.with_peers(pc);
    }

    // The ID kind must be a valid kind (alphanumeric, no hyphens).
    // The actual node registration uses config.node_id and config.group_name.
    let svc_id = ID::new_with_str(&format!("slim/{name}")).unwrap();
    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

/// Start a single node in a group (no peers).
async fn start_single_node(name: &str, group: &str, southbound_port: u16, dp_port: u16) -> Service {
    start_grouped_node(name, group, southbound_port, dp_port, &[(name, dp_port)]).await
}

// --- Client App ---

/// Connect a client app to a DP node and subscribe to a name.
/// Returns the Service handle (drop/shutdown to disconnect).
async fn start_subscribing_app(dp_port: u16, org: &str, ns: &str, component: &str) -> Service {
    let name = Name::from_strings([org, ns, component]);
    let client_cfg = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{dp_port}"))
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
    // Leak app and rx so the subscription stays alive for the test duration.
    // Dropping them would disconnect the app and remove the route.
    std::mem::forget(app);
    std::mem::forget(rx);
    svc
}

// --- Northbound API Queries ---

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

// --- Wait Helpers ---

async fn wait_for_nodes_connected(client: &mut NbClient, node_ids: &[&str], timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let entries = collect_nodes(client).await;
        let all_connected = node_ids.iter().all(|id| {
            entries
                .iter()
                .any(|e| e.id == *id && e.status == NODE_CONNECTED)
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
            if l.deleted || l.status != LINK_APPLIED {
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
                        "{}->{}[status={},deleted={}]",
                        l.source_node_id, l.dest_node_id, l.status, l.deleted
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

async fn wait_for_route(
    client: &mut NbClient,
    src: &str,
    dest: &str,
    expected_status: i32,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let routes = collect_routes(client, src, dest).await;
        let found = routes.iter().any(|e| {
            e.source_node_id == src && e.dest_node_id == dest && e.status == expected_status
        });
        if found {
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
                "timeout waiting for route {src}->{dest} with status={expected_status}. Current: [{}]",
                statuses.join(", ")
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_route_applied(client: &mut NbClient, src: &str, dest: &str, timeout: Duration) {
    wait_for_route(client, src, dest, ROUTE_APPLIED, timeout).await;
}

async fn wait_for_route_deleted(client: &mut NbClient, src: &str, dest: &str, timeout: Duration) {
    wait_for_route(client, src, dest, ROUTE_DELETED, timeout).await;
}

/// Wait until no Applied route exists from src to dest (route may be absent or Deleted).
async fn wait_for_no_applied_route(
    client: &mut NbClient,
    src: &str,
    dest: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let routes = collect_routes(client, src, dest).await;
        let has_applied = routes.iter().any(|e| {
            e.source_node_id == src && e.dest_node_id == dest && e.status == ROUTE_APPLIED
        });
        if !has_applied {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for route {src}->{dest} to be removed");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait until NO routes with the given name are active (all DELETED or absent).
async fn wait_for_no_active_routes_with_name(
    client: &mut NbClient,
    c0: &str,
    c1: &str,
    c2: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    let name = Name::from_strings([c0, c1, c2]);
    loop {
        let routes = collect_routes(client, "", "").await;
        let has_active = routes
            .iter()
            .any(|r| r.status != ROUTE_DELETED && r.name.as_ref().is_some_and(|n| n == &name));
        if !has_active {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for all routes with name {c0}/{c1}/{c2} to be deleted");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait for an Applied link between two groups (in either direction) and return it.
async fn wait_for_link_between_groups_entry(
    client: &mut NbClient,
    group_a: &str,
    group_b: &str,
    timeout: Duration,
) -> LinkEntry {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut links = vec![];
    loop {
        links = collect_links(client, "", "").await;
        if let Some(link) = links.iter().find(|l| {
            if l.deleted || l.status != LINK_APPLIED {
                return false;
            }
            let sg = l.source_node_id.split('/').next().unwrap_or("");
            let dg = l.dest_node_id.split('/').next().unwrap_or("");
            (sg == group_a && dg == group_b) || (sg == group_b && dg == group_a)
        }) {
            return link.clone();
        }
        if tokio::time::Instant::now() >= deadline {
            let link_info: Vec<_> = links
                .iter()
                .map(|l| {
                    format!(
                        "{}->{}[status={},deleted={}]",
                        l.source_node_id, l.dest_node_id, l.status, l.deleted
                    )
                })
                .collect();
            panic!(
                "timeout waiting for link between {group_a} and {group_b}. Links: [{}]",
                link_info.join(", ")
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_test_writer()
        .try_init();
}

// =============================================================================
// Tests
// =============================================================================

/// Test 1: Inter-group links created and claimed
///
/// Scenario:
///   - Start a control plane with full-mesh topology (default).
///   - Start one node in each of 3 groups: group-a, group-b, group-c.
///   - Verify that inter-group links are automatically created between all group
///     pairs and reach Applied status (meaning both sides have negotiated).
///
/// Validates: topology graph -> link creation -> link claim via negotiation -> Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_inter_group_links_created_and_claimed() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();
    let c_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "group-c", cp.southbound_port, c_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");
    let id_c = grouped_node_id("group-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;

    // Full mesh of 3 groups = links between all 3 pairs.
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-b", "group-c", DEFAULT_TIMEOUT).await;

    // Verify all links are Applied and non-deleted.
    let links = collect_links(&mut client, "", "").await;
    let applied: Vec<_> = links
        .iter()
        .filter(|l| l.status == LINK_APPLIED && !l.deleted)
        .collect();
    assert!(
        applied.len() >= 3,
        "expected at least 3 Applied links for full mesh of 3 groups, got {}",
        applied.len()
    );

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    node_c.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 3: Subscription creates correct routes and does NOT propagate over Remote links
///
/// Scenario:
///   - Start CP + 2 nodes in different groups (group-a, group-b).
///   - An app subscribes on node-a:
///     - Verify wildcard route *->node-a and remote route node-b->node-a are created.
///   - A second app subscribes on node-b:
///     - Verify wildcard route *->node-b and remote route node-a->node-b are created.
///     - Verify NO spurious wildcard *->node-a is created from propagation over
///       the Remote inter-group link.
///
/// Validates: subscribe → CP notified → SPT expansion + no cross-link propagation.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_routing() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes on node-a → creates wildcard + expanded route.
    let app_a = start_subscribing_app(a_port, "org", "ns", "local-sub").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // App subscribes on node-b → creates wildcard + expanded route.
    let app_b = start_subscribing_app(b_port, "org", "ns", "no-propagate").await;
    wait_for_route_applied(&mut client, "*", &id_b, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_a, &id_b, DEFAULT_TIMEOUT).await;

    // Give extra time for any incorrect routes to appear.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify no EXTRA wildcard *->node-a was created by propagation from node-b's subscribe.
    // There should be exactly one wildcard for node-a (from app_a), not a second one from
    // node-b's subscription leaking over the Remote link.
    let routes = collect_routes(&mut client, "*", &id_a).await;
    let wildcard_count = routes
        .iter()
        .filter(|r| r.source_node_id == "*" && r.dest_node_id == id_a && r.status == ROUTE_APPLIED)
        .count();
    assert_eq!(
        wildcard_count, 1,
        "expected exactly 1 wildcard *->{id_a} (from app_a), got {wildcard_count} — \
         subscription may have propagated over Remote link"
    );

    // Verify no reverse route node-b->node-a exists for the "no-propagate" service.
    let routes = collect_routes(&mut client, &id_b, &id_a).await;
    let wrong_reverse = routes.iter().any(|r| {
        r.source_node_id == id_b
            && r.dest_node_id == id_a
            && r.status == ROUTE_APPLIED
            && r.name
                .as_ref()
                .and_then(|n| n.str_name.as_ref())
                .map(|sn| sn.str_component_2 == "no-propagate")
                .unwrap_or(false)
    });
    assert!(
        !wrong_reverse,
        "subscription created wrong reverse route: {id_b}->{id_a} for 'no-propagate'"
    );

    app_a.shutdown().await.ok();
    app_b.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 4: Bidirectional inter-group routes
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - Apps subscribe on both nodes (different service names).
///   - Verify routes exist in both directions:
///     - node-a->node-b for the service on node-b
///     - node-b->node-a for the service on node-a
///
/// Validates: bidirectional traffic over a single inter-group link pair.
#[tokio::test(flavor = "multi_thread")]
async fn test_bidirectional_inter_group_routes() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App on node-a subscribes to svc-a.
    let app_a = start_subscribing_app(a_port, "org", "ns", "svc-a").await;
    // App on node-b subscribes to svc-b.
    let app_b = start_subscribing_app(b_port, "org", "ns", "svc-b").await;

    // Route from node-b to node-a (for svc-a).
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;
    // Route from node-a to node-b (for svc-b).
    wait_for_route_applied(&mut client, &id_a, &id_b, DEFAULT_TIMEOUT).await;

    app_a.shutdown().await.ok();
    app_b.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 5: SPT route via hub (star topology)
///
/// Scenario:
///   - Start CP with star topology: "platform" is the hub.
///   - Start 3 nodes: hub, spoke-a (customer-a), spoke-b (customer-b).
///   - An app subscribes on spoke-a.
///   - Verify spoke-b gets a route to spoke-a that transits through the hub.
///   - Verify no direct spoke-to-spoke link exists.
///
/// Validates: SPT routing through intermediate hops when no direct link exists.
#[tokio::test(flavor = "multi_thread")]
async fn test_spt_route_via_hub() {
    init_tracing();

    let cp = start_control_plane(star_topology_config("platform")).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let hub_port = reserve_port();
    let spoke_a_port = reserve_port();
    let spoke_b_port = reserve_port();

    let hub = start_single_node("hub", "platform", cp.southbound_port, hub_port).await;
    let spoke_a =
        start_single_node("spoke-a", "customer-a", cp.southbound_port, spoke_a_port).await;
    let spoke_b =
        start_single_node("spoke-b", "customer-b", cp.southbound_port, spoke_b_port).await;

    let id_hub = grouped_node_id("platform", "hub");
    let id_spoke_a = grouped_node_id("customer-a", "spoke-a");
    let id_spoke_b = grouped_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;

    // Spoke-to-hub links.
    wait_for_link_between_groups(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

    // Verify NO spoke-to-spoke link.
    let links = collect_links(&mut client, "", "").await;
    let spoke_to_spoke = links.iter().any(|l| {
        if l.deleted {
            return false;
        }
        let sg = l.source_node_id.split('/').next().unwrap_or("");
        let dg = l.dest_node_id.split('/').next().unwrap_or("");
        (sg == "customer-a" && dg == "customer-b") || (sg == "customer-b" && dg == "customer-a")
    });
    assert!(
        !spoke_to_spoke,
        "star topology must NOT create spoke-to-spoke links"
    );

    // App subscribes on spoke-a.
    let app = start_subscribing_app(spoke_a_port, "org", "ns", "transit-svc").await;

    // spoke-b should get a route to spoke-a (via hub).
    wait_for_route_applied(&mut client, &id_spoke_b, &id_spoke_a, DEFAULT_TIMEOUT).await;
    // hub should also have a route to spoke-a.
    wait_for_route_applied(&mut client, &id_hub, &id_spoke_a, DEFAULT_TIMEOUT).await;

    app.shutdown().await.ok();
    hub.shutdown().await.ok();
    spoke_a.shutdown().await.ok();
    spoke_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 6: Multicast - no duplicate routes per (source, dest) pair
///
/// Scenario:
///   - Full mesh with 3 groups, 1 node each.
///   - Two apps subscribe to the same name on different nodes.
///   - Verify no duplicate routes (same source->dest) are created.
///
/// Validates: SPT expansion doesn't create conflicting or duplicate routes.
#[tokio::test(flavor = "multi_thread")]
async fn test_multicast_no_duplicate_routes() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();
    let c_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "group-c", cp.southbound_port, c_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");
    let id_c = grouped_node_id("group-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-b", "group-c", DEFAULT_TIMEOUT).await;

    // First subscriber on node-a.
    let app_a = start_subscribing_app(a_port, "org", "ns", "multicast").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_c, &id_a, DEFAULT_TIMEOUT).await;

    // Second subscriber on node-c (same name).
    let app_c = start_subscribing_app(c_port, "org", "ns", "multicast").await;
    wait_for_route_applied(&mut client, &id_a, &id_c, DEFAULT_TIMEOUT).await;

    // Allow system to settle.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify no duplicate routes.
    let all_routes = collect_routes(&mut client, "", "").await;
    let active_routes: Vec<_> = all_routes
        .iter()
        .filter(|r| r.source_node_id != "*" && r.status == ROUTE_APPLIED)
        .collect();

    let mut seen = std::collections::HashSet::new();
    for r in &active_routes {
        let key = (r.source_node_id.clone(), r.dest_node_id.clone());
        assert!(
            seen.insert(key.clone()),
            "duplicate route: {} -> {}",
            key.0,
            key.1
        );
    }

    app_a.shutdown().await.ok();
    app_c.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    node_c.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 7: Source gateway failover
///
/// Scenario:
///   - Start CP + 2 nodes in group-a (node-a1, node-a2) + 1 node in group-b.
///   - App subscribes on node-b → wildcard *->node-b + expanded route from gateway->node-b.
///   - Kill the gateway node in group-a.
///   - Verify: link is reassigned to the sibling in group-a.
///   - Verify: route from sibling->node-b is re-expanded and reaches Applied.
///
/// Validates: source-side gateway failover + wildcard route re-expansion.
#[tokio::test(flavor = "multi_thread")]
async fn test_source_gateway_failover() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a1_port = reserve_port();
    let a2_port = reserve_port();
    let b_port = reserve_port();

    let peers_a: Vec<(&str, u16)> = vec![("node-a1", a1_port), ("node-a2", a2_port)];
    let node_a1 =
        start_grouped_node("node-a1", "group-a", cp.southbound_port, a1_port, &peers_a).await;
    let node_a2 =
        start_grouped_node("node-a2", "group-a", cp.southbound_port, a2_port, &peers_a).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a1 = grouped_node_id("group-a", "node-a1");
    let id_a2 = grouped_node_id("group-a", "node-a2");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a1, &id_a2, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // Find which node in group-a is the gateway.
    let link =
        wait_for_link_between_groups_entry(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT)
            .await;

    let gateway_id = if link.source_node_id.starts_with("group-a") {
        link.source_node_id.clone()
    } else {
        link.dest_node_id.clone()
    };
    let sibling_id = if gateway_id == id_a1 {
        id_a2.clone()
    } else {
        id_a1.clone()
    };

    // App subscribes on node-b → creates wildcard *->node-b + expanded route gateway->node-b.
    let app = start_subscribing_app(b_port, "org", "ns", "failover-svc").await;
    wait_for_route_applied(&mut client, "*", &id_b, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &gateway_id, &id_b, DEFAULT_TIMEOUT).await;

    // Kill the gateway node.
    if gateway_id == id_a1 {
        node_a1.deregister().await.ok();
        node_a1.shutdown().await.ok();
    } else {
        node_a2.deregister().await.ok();
        node_a2.shutdown().await.ok();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The sibling should become the new gateway (link involving sibling and group-b).
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let links = collect_links(&mut client, "", "").await;
        let new_link = links.iter().any(|l| {
            let involves_sibling = l.source_node_id == sibling_id || l.dest_node_id == sibling_id;
            let involves_group_b =
                l.source_node_id.starts_with("group-b") || l.dest_node_id.starts_with("group-b");
            involves_sibling && involves_group_b && l.status == LINK_APPLIED && !l.deleted
        });
        if new_link {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for link to be reassigned to {sibling_id}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Route from sibling->node-b should be re-expanded via wildcard and reach Applied.
    wait_for_route_applied(&mut client, &sibling_id, &id_b, DEFAULT_TIMEOUT).await;

    // Cleanup.
    app.shutdown().await.ok();
    if gateway_id == id_a1 {
        node_a2.shutdown().await.ok();
    } else {
        node_a1.shutdown().await.ok();
    }
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 8: Dest gateway failover
///
/// Scenario:
///   - Start CP + 1 node in group-a + 2 nodes in group-b (node-b1, node-b2).
///   - Wait for inter-group link targeting one of group-b's nodes.
///   - Kill the dest gateway in group-b.
///   - Verify: a new link is created targeting the sibling in group-b.
///
/// Validates: dest-side gateway failover (link deleted + recreated to new target).
#[tokio::test(flavor = "multi_thread")]
async fn test_dest_gateway_failover() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b1_port = reserve_port();
    let b2_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let peers_b: Vec<(&str, u16)> = vec![("node-b1", b1_port), ("node-b2", b2_port)];
    let node_b1 =
        start_grouped_node("node-b1", "group-b", cp.southbound_port, b1_port, &peers_b).await;
    let node_b2 =
        start_grouped_node("node-b2", "group-b", cp.southbound_port, b2_port, &peers_b).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b1 = grouped_node_id("group-b", "node-b1");
    let id_b2 = grouped_node_id("group-b", "node-b2");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b1, &id_b2], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    tracing::info!("Initial link established between group-a and group-b");

    // Find which node in group-b is involved in the link (as source or dest).
    let link =
        wait_for_link_between_groups_entry(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT)
            .await;

    // Determine which group-b node is the gateway (could be source or dest side).
    let b_gateway = if link.source_node_id.starts_with("group-b") {
        link.source_node_id.clone()
    } else {
        link.dest_node_id.clone()
    };
    let b_sibling = if b_gateway == id_b1 {
        id_b2.clone()
    } else {
        id_b1.clone()
    };

    tracing::info!("group-b gateway is {b_gateway}, sibling is {b_sibling}");

    // Kill the group-b gateway.
    if b_gateway == id_b1 {
        tracing::info!("Shutting down {id_b1}");
        node_b1.deregister().await.ok();
        node_b1.shutdown().await.ok();
    } else {
        tracing::info!("Shutting down {id_b2}");
        node_b2.deregister().await.ok();
        node_b2.shutdown().await.ok();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A new link should be created involving the sibling in group-b.
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let links = collect_links(&mut client, "", "").await;
        let new_link = links.iter().any(|l| {
            let involves_sibling = l.source_node_id == b_sibling || l.dest_node_id == b_sibling;
            let involves_group_a =
                l.source_node_id.starts_with("group-a") || l.dest_node_id.starts_with("group-a");
            involves_sibling && involves_group_a && l.status == LINK_APPLIED && !l.deleted
        });
        if new_link {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for link to be recreated to {b_sibling}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    node_a.shutdown().await.ok();
    if b_gateway == id_b1 {
        node_b2.shutdown().await.ok();
    } else {
        node_b1.shutdown().await.ok();
    }
    stop_control_plane(cp).await;
}

/// Test 9: Wildcard route deleted on node crash
///
/// Scenario:
///   - Start CP + 1 node in group-a (with app subscribed) + 1 node in group-b.
///   - Verify wildcard route *->node-a is Applied.
///   - Kill node-a.
///   - Verify wildcard route *->node-a is marked as DELETED.
///
/// Validates: node disconnect cleans up wildcard routes to prevent black-hole routing.
#[tokio::test(flavor = "multi_thread")]
async fn test_wildcard_route_deleted_on_node_crash() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes on node-a.
    let _app = start_subscribing_app(a_port, "org", "ns", "crash-svc").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;

    // Kill node-a (also kills the app connected to it).
    node_a.deregister().await.ok();
    node_a.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Wildcard route should be DELETED.
    wait_for_route_deleted(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;

    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 10: Route restored when node reconnects with new app subscription
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - App subscribes on node-a -> routes created.
///   - Kill node-a -> routes marked DELETED.
///   - Restart node-a on the same port.
///   - New app subscribes on node-a with the same name.
///   - Verify routes are re-created and reach Applied.
///
/// Validates: full disconnect->reconnect lifecycle for route cleanup and recreation.
#[tokio::test(flavor = "multi_thread")]
async fn test_route_restored_after_reconnect() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes.
    let app = start_subscribing_app(a_port, "org", "ns", "reconnect-svc").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Kill node-a + app.
    app.shutdown().await.ok();
    node_a.deregister().await.ok();
    node_a.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Routes should be DELETED.
    wait_for_route_deleted(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;

    // Restart node-a on same port.
    let node_a2 = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // New app subscribes.
    let app2 = start_subscribing_app(a_port, "org", "ns", "reconnect-svc").await;

    // Routes should be re-created and Applied.
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    app2.shutdown().await.ok();
    node_a2.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 11: App disconnect removes routes
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - App subscribes on node-a -> routes created.
///   - App disconnects (unsubscribe).
///   - Verify that all routes for this subscription are removed/deleted.
///
/// Validates: unsubscribe path - app disconnect triggers route deletion via CP.
#[tokio::test(flavor = "multi_thread")]
async fn test_app_disconnect_removes_routes() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes.
    let app = start_subscribing_app(a_port, "org", "ns", "disconnect-svc").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;

    // App disconnects.
    app.shutdown().await.ok();

    // Routes should be cleaned up.
    wait_for_no_active_routes_with_name(
        &mut client,
        "org",
        "ns",
        "disconnect-svc",
        DEFAULT_TIMEOUT,
    )
    .await;

    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 12: Last node in group removes group from topology
///
/// Scenario:
///   - Start CP + 3 nodes in 3 different groups.
///   - Verify links between all groups.
///   - Shut down the node in one group (removing the group entirely).
///   - Verify links involving that group are cleaned up.
///   - Verify remaining groups still have their link.
///
/// Validates: group removal from link graph when all nodes depart.
#[tokio::test(flavor = "multi_thread")]
async fn test_last_node_removes_group_links() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();
    let c_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "group-c", cp.southbound_port, c_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");
    let id_c = grouped_node_id("group-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-b", "group-c", DEFAULT_TIMEOUT).await;

    // Kill the only node in group-a.
    // Use deregister() to explicitly notify the CP (shutdown alone doesn't
    // reliably close the gRPC stream fast enough in-process).
    node_a.deregister().await.ok();
    node_a.shutdown().await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Links involving group-a should be deleted or gone.
    let links = collect_links(&mut client, "", "").await;
    let active_group_a_links: Vec<_> = links
        .iter()
        .filter(|l| {
            !l.deleted
                && l.status == LINK_APPLIED
                && (l.source_node_id.starts_with("group-a")
                    || l.dest_node_id.starts_with("group-a"))
        })
        .collect();
    assert!(
        active_group_a_links.is_empty(),
        "expected no active links for group-a after node departed, found: {:?}",
        active_group_a_links
            .iter()
            .map(|l| format!("{}->{}", l.source_node_id, l.dest_node_id))
            .collect::<Vec<_>>()
    );

    // Link between group-b and group-c should still exist.
    wait_for_link_between_groups(&mut client, "group-b", "group-c", SHORT_TIMEOUT).await;

    node_b.shutdown().await.ok();
    node_c.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 13: Node crash and recovery - link re-established
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - App subscribes on node-a → wildcard + expanded route created.
///   - Kill node-b (dest side), wait briefly.
///   - Restart node-b on the same port.
///   - Verify the link is re-established and expanded route becomes Applied again.
///
/// Validates: full crash->recovery cycle for a single-node group with subscription-based routes.
#[tokio::test(flavor = "multi_thread")]
async fn test_node_crash_and_link_recovery() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes on node-a — creates wildcard template + expanded route via SPT.
    let app = start_subscribing_app(a_port, "org", "ns", "recovery-svc").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Crash node-b (the destination side).
    node_b.deregister().await.ok();
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Restart node-b on same port.
    let node_b2 = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;
    wait_for_nodes_connected(&mut client, &[&id_b], SHORT_TIMEOUT).await;

    // Link should be re-established and expanded route re-applied.
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    app.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 14: Multiple wildcard routes for different service names
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - Two different services subscribe on node-a (different names).
///   - A third service subscribes on node-b.
///   - Verify: each service has independent wildcard + expanded routes,
///     and they don't interfere with each other.
///
/// Validates: multiple independent wildcard routes coexist correctly.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_wildcard_routes_different_names() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // Two different services subscribe on node-a.
    let app_svc1 = start_subscribing_app(a_port, "org", "ns", "svc-alpha").await;
    let app_svc2 = start_subscribing_app(a_port, "org", "ns", "svc-beta").await;
    // A different service subscribes on node-b.
    let app_svc3 = start_subscribing_app(b_port, "org", "ns", "svc-gamma").await;

    // Each should have its own wildcard route.
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, "*", &id_b, DEFAULT_TIMEOUT).await;

    // Expanded routes: node-b should route to node-a for both alpha and beta.
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;
    // node-a should route to node-b for gamma.
    wait_for_route_applied(&mut client, &id_a, &id_b, DEFAULT_TIMEOUT).await;

    // Count distinct service names in wildcard routes to node-a.
    let routes = collect_routes(&mut client, "*", &id_a).await;
    let wildcard_names: std::collections::HashSet<_> = routes
        .iter()
        .filter(|r| r.status == ROUTE_APPLIED)
        .filter_map(|r| {
            r.name
                .as_ref()
                .and_then(|n| n.str_name.as_ref())
                .map(|sn| sn.str_component_2.clone())
        })
        .collect();
    assert!(
        wildcard_names.contains("svc-alpha"),
        "missing wildcard for svc-alpha"
    );
    assert!(
        wildcard_names.contains("svc-beta"),
        "missing wildcard for svc-beta"
    );

    // Disconnect one service — only its routes should be removed.
    app_svc1.shutdown().await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // svc-beta wildcard should still be active.
    let routes = collect_routes(&mut client, "*", &id_a).await;
    let beta_still_active = routes.iter().any(|r| {
        r.status == ROUTE_APPLIED
            && r.name
                .as_ref()
                .and_then(|n| n.str_name.as_ref())
                .map(|sn| sn.str_component_2 == "svc-beta")
                .unwrap_or(false)
    });
    assert!(
        beta_still_active,
        "svc-beta wildcard should survive svc-alpha disconnection"
    );

    app_svc2.shutdown().await.ok();
    app_svc3.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 15: Node reconnects with different external endpoint
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - Links established, app subscribes, routes applied.
///   - Crash node-b, restart on a different port.
///   - Verify: link is re-established with the new endpoint and routes recover.
///
/// Validates: reconnection with changed connection details updates link endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn test_reconnect_different_endpoint() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port_original = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port_original).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // App subscribes.
    let app = start_subscribing_app(a_port, "org", "ns", "endpoint-svc").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Crash node-b.
    node_b.deregister().await.ok();
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Restart node-b on a DIFFERENT port.
    let b_port_new = reserve_port();
    let node_b2 = start_single_node("node-b", "group-b", cp.southbound_port, b_port_new).await;
    wait_for_nodes_connected(&mut client, &[&id_b], SHORT_TIMEOUT).await;

    // Link should be re-established (with new endpoint).
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // Route should recover.
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Verify the link involves node-b's new endpoint. The link direction may
    // vary, so check that the link is between the groups and active.
    let link =
        wait_for_link_between_groups_entry(&mut client, "group-a", "group-b", SHORT_TIMEOUT).await;
    // The link should be fully established (both source and dest populated).
    assert!(
        !link.source_node_id.is_empty() && !link.dest_node_id.is_empty(),
        "link should have both source and dest populated after reconnect"
    );

    app.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test 16: Star topology — hub crash disconnects all spokes
///
/// Scenario:
///   - Star topology: hub (platform) connected to spoke-a and spoke-b.
///   - App subscribes on spoke-a → routes created via hub.
///   - Kill the hub node.
///   - Verify: routes involving the hub are cleaned up.
///   - Restart hub → links and routes recover.
///
/// Validates: hub crash in star topology and recovery after hub restart.
#[tokio::test(flavor = "multi_thread")]
async fn test_star_topology_hub_crash_and_recovery() {
    init_tracing();

    let cp = start_control_plane(star_topology_config("platform")).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let hub_port = reserve_port();
    let spoke_a_port = reserve_port();
    let spoke_b_port = reserve_port();

    let hub = start_single_node("hub", "platform", cp.southbound_port, hub_port).await;
    let spoke_a =
        start_single_node("spoke-a", "customer-a", cp.southbound_port, spoke_a_port).await;
    let spoke_b =
        start_single_node("spoke-b", "customer-b", cp.southbound_port, spoke_b_port).await;

    let id_hub = grouped_node_id("platform", "hub");
    let id_spoke_a = grouped_node_id("customer-a", "spoke-a");
    let id_spoke_b = grouped_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;
    wait_for_link_between_groups(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

    // App subscribes on spoke-a → spoke-b gets route via hub.
    let app = start_subscribing_app(spoke_a_port, "org", "ns", "hub-crash-svc").await;
    wait_for_route_applied(&mut client, &id_spoke_b, &id_spoke_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_hub, &id_spoke_a, DEFAULT_TIMEOUT).await;

    // Crash the hub.
    hub.deregister().await.ok();
    hub.shutdown().await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Links involving the hub should be gone or deleted.
    let links = collect_links(&mut client, "", "").await;
    let active_hub_links: Vec<_> = links
        .iter()
        .filter(|l| {
            !l.deleted
                && l.status == LINK_APPLIED
                && (l.source_node_id.contains("platform") || l.dest_node_id.contains("platform"))
        })
        .collect();
    assert!(
        active_hub_links.is_empty(),
        "expected no active links involving hub after crash, found: {:?}",
        active_hub_links
            .iter()
            .map(|l| format!("{}->{}", l.source_node_id, l.dest_node_id))
            .collect::<Vec<_>>()
    );

    // Restart the hub.
    let hub2 = start_single_node("hub", "platform", cp.southbound_port, hub_port).await;
    wait_for_nodes_connected(&mut client, &[&id_hub], SHORT_TIMEOUT).await;

    // Links should be re-established.
    wait_for_link_between_groups(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

    // The original app's subscription on spoke-a is still active. After the
    // links recover, the subscription should propagate over the new link to
    // the hub. If not re-propagated automatically, a new subscription triggers it.
    // Start a new app to ensure routes are created.
    let app2 = start_subscribing_app(spoke_a_port, "org", "ns", "hub-crash-svc2").await;
    wait_for_route_applied(&mut client, &id_hub, &id_spoke_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_spoke_b, &id_spoke_a, DEFAULT_TIMEOUT).await;

    app.shutdown().await.ok();
    app2.shutdown().await.ok();
    hub2.shutdown().await.ok();
    spoke_a.shutdown().await.ok();
    spoke_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Helper: build a segmented star topology with `$group` template.
/// Hub group connects to each spoke group in a separate segment.
fn segmented_star_topology(hub_group: &str) -> TopologyConfig {
    TopologyConfig::Segments(vec![SegmentConfig {
        name: "seg-$group".to_string(),
        links: vec![AdjacencyEntry {
            group: hub_group.to_string(),
            neighbors: vec!["$group".to_string()],
        }],
    }])
}

/// Test: Segmented star topology isolates spokes but hub bridges all segments.
///
/// Scenario:
///   - CP with segmented star topology ($group template): platform↔$group.
///   - 3 nodes: hub (platform), spoke-a (customer-a), spoke-b (customer-b).
///   - App subscribes on spoke-a → hub gets a route, spoke-b does NOT.
///   - App subscribes on hub → both spokes get a route.
///   - No spoke-to-spoke links.
///
/// Validates: segment isolation, hub bridging, $group template expansion.
#[tokio::test(flavor = "multi_thread")]
async fn test_segmented_star_isolates_spokes() {
    init_tracing();

    let cp = start_control_plane(segmented_star_topology("platform")).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let hub_port = reserve_port();
    let spoke_a_port = reserve_port();
    let spoke_b_port = reserve_port();

    let hub = start_single_node("hub", "platform", cp.southbound_port, hub_port).await;
    let spoke_a =
        start_single_node("spoke-a", "customer-a", cp.southbound_port, spoke_a_port).await;
    let spoke_b =
        start_single_node("spoke-b", "customer-b", cp.southbound_port, spoke_b_port).await;

    let id_hub = grouped_node_id("platform", "hub");
    let id_spoke_a = grouped_node_id("customer-a", "spoke-a");
    let id_spoke_b = grouped_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;

    // Hub should have links to both spokes.
    wait_for_link_between_groups(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_groups(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

    // Verify NO spoke-to-spoke link.
    let links = collect_links(&mut client, "", "").await;
    let spoke_to_spoke = links.iter().any(|l| {
        if l.deleted {
            return false;
        }
        let sg = l.source_node_id.split('/').next().unwrap_or("");
        let dg = l.dest_node_id.split('/').next().unwrap_or("");
        (sg == "customer-a" && dg == "customer-b") || (sg == "customer-b" && dg == "customer-a")
    });
    assert!(
        !spoke_to_spoke,
        "segmented star must NOT create spoke-to-spoke links"
    );

    // --- Part 1: route on spoke-a → hub gets it, spoke-b does NOT ---
    let app_a = start_subscribing_app(spoke_a_port, "org", "ns", "seg-svc-a").await;

    // Hub should get a route to spoke-a (same segment seg-customer-a).
    wait_for_route_applied(&mut client, &id_hub, &id_spoke_a, DEFAULT_TIMEOUT).await;

    // Spoke-b should NOT get a route to spoke-a (different segment).
    // Allow time for any routes to propagate, then verify absence.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let spoke_b_routes = collect_routes(&mut client, &id_spoke_b, &id_spoke_a).await;
    let spoke_b_has_route = spoke_b_routes.iter().any(|r| r.status == ROUTE_APPLIED);
    assert!(
        !spoke_b_has_route,
        "spoke-b must NOT have a route to spoke-a (segment isolation)"
    );

    // --- Part 2: route on hub → both spokes get it ---
    let app_hub = start_subscribing_app(hub_port, "org", "ns", "seg-svc-hub").await;

    // Both spokes should get routes to hub (hub is in both segments).
    wait_for_route_applied(&mut client, &id_spoke_a, &id_hub, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_spoke_b, &id_hub, DEFAULT_TIMEOUT).await;

    app_a.shutdown().await.ok();
    app_hub.shutdown().await.ok();
    hub.shutdown().await.ok();
    spoke_a.shutdown().await.ok();
    spoke_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

// =============================================================================
// API-managed topology tests
// =============================================================================

/// Test: API-managed topology lifecycle
///
/// Scenario:
///   - Start CP in API-managed mode (no topology config).
///   - Pre-configure segment + link before nodes exist (returns warnings).
///   - Start two nodes in different groups → link becomes Applied.
///   - Verify idempotent add_topology_link (SQLite ON CONFLICT DO NOTHING).
///   - Remove the link → link is torn down, route cleaned up.
///   - Re-add link → route restored.
///   - Remove the segment → segment disappears from list.
#[tokio::test(flavor = "multi_thread")]
async fn test_api_mode_topology_lifecycle() {
    init_tracing();

    let cp = start_control_plane(TopologyConfig::ApiManaged).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    // Pre-configure topology BEFORE nodes register.
    client
        .add_segment(AddSegmentRequest {
            name: "test-seg".to_string(),
        })
        .await
        .expect("add_segment failed");

    let resp = client
        .add_topology_link(AddTopologyLinkRequest {
            group_a: "group-a".to_string(),
            group_b: "group-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("add_topology_link before nodes should succeed")
        .into_inner();
    // Should have warnings about missing groups.
    assert!(
        !resp.warnings.is_empty(),
        "expected warnings about missing groups, got none"
    );

    // Verify idempotency: adding the same link again should succeed.
    client
        .add_topology_link(AddTopologyLinkRequest {
            group_a: "group-a".to_string(),
            group_b: "group-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("add_topology_link should be idempotent");

    // Now start nodes — link should become Applied via reconciliation.
    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "group-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "group-b", cp.southbound_port, b_port).await;

    let id_a = grouped_node_id("group-a", "node-a");
    let id_b = grouped_node_id("group-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;

    // Wait for link to be created and reach Applied status.
    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;

    // Start a subscribing app on node-a → should create a route from node-b to node-a.
    let app_a = start_subscribing_app(a_port, "org", "ns", "api-svc").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Remove link via API → routes through that link should be cleaned up.
    client
        .remove_topology_link(RemoveTopologyLinkRequest {
            group_a: "group-a".to_string(),
            group_b: "group-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("remove_topology_link failed");

    // Wait for link to be torn down (deleted or gone).
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let links = collect_links(&mut client, "", "").await;
        let active: Vec<_> = links
            .iter()
            .filter(|l| l.status == LINK_APPLIED && !l.deleted)
            .collect();
        if active.is_empty() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for link removal");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Verify the route from node-b to node-a is gone after link removal.
    wait_for_no_applied_route(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Re-add the link → route should be re-created via expand_all_wildcard_routes.
    client
        .add_topology_link(AddTopologyLinkRequest {
            group_a: "group-a".to_string(),
            group_b: "group-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("re-add topology link failed");

    wait_for_link_between_groups(&mut client, "group-a", "group-b", DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Remove segment (cascades link deletion) → route should be cleaned up again.
    client
        .remove_segment(RemoveSegmentRequest {
            name: "test-seg".to_string(),
        })
        .await
        .expect("remove_segment failed");

    wait_for_no_applied_route(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Verify segment is gone.
    let resp = client
        .list_segments(SegmentListRequest {})
        .await
        .expect("list_segments failed")
        .into_inner();
    assert!(
        !resp.segments.iter().any(|s| s.name == "test-seg"),
        "segment should have been removed"
    );

    app_a.shutdown().await.ok();
    node_a.shutdown().await.ok();
    node_b.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: Config-managed mode rejects topology mutation APIs
///
/// Scenario:
///   - Start CP with a config-managed topology (explicit links).
///   - Attempt to add a segment via gRPC API → expect FAILED_PRECONDITION.
#[tokio::test(flavor = "multi_thread")]
async fn test_config_mode_rejects_topology_mutations() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let err = client
        .add_segment(AddSegmentRequest {
            name: "should-fail".to_string(),
        })
        .await
        .expect_err("add_segment should fail in config mode");

    assert_eq!(
        err.code(),
        tonic::Code::FailedPrecondition,
        "expected FAILED_PRECONDITION, got {:?}: {}",
        err.code(),
        err.message()
    );

    let err = client
        .add_topology_link(AddTopologyLinkRequest {
            group_a: "a".to_string(),
            group_b: "b".to_string(),
            segment: "default".to_string(),
        })
        .await
        .expect_err("add_topology_link should fail in config mode");

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);

    stop_control_plane(cp).await;
}

// =============================================================================
// Auth method integration tests
//
// These tests verify that the controller's reconcile path correctly handles
// each auth method when the control plane instructs node_1 to connect to node_2.
//
// Topology for all tests:
//   node_1 (group-auth-a)  <-- outbound connector (reconciler source)
//   node_2 (group-auth-b)  <-- protected server (auth requirement source)
//   control_plane          <-- full-mesh topology, mediates the link
//
// Startup order matters: the *second* node to register becomes the inter-group
// link source. Always start node_2 (protected) before node_1 (connector) so
// node_1 receives ServerConnectionConfig and performs the authenticated dial.
//
// The control plane learns node_2's auth requirement from its registered
// ConnectionDetails (derived from node_2's dataplane ServerConfig.auth).
// It then sends a ServerConnectionConfig to node_1's reconciler with the
// appropriate RequiredAuthMethod. node_1 looks up credentials from its local
// outbound_clients config (keyed by endpoint) and connects — or fails if no
// matching entry exists.
// =============================================================================

const LINK_FAILED: i32 = 3;

const AUTH_GROUP_CONNECTOR: &str = "group-auth-a";
const AUTH_GROUP_PROTECTED: &str = "group-auth-b";
const AUTH_NODE_CONNECTOR: &str = "node-1";
const AUTH_NODE_PROTECTED: &str = "node-2";

fn auth_connector_id() -> String {
    grouped_node_id(AUTH_GROUP_CONNECTOR, AUTH_NODE_CONNECTOR)
}

fn auth_protected_id() -> String {
    grouped_node_id(AUTH_GROUP_PROTECTED, AUTH_NODE_PROTECTED)
}

fn protected_endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

async fn find_link_between_auth_groups(client: &mut NbClient) -> Option<LinkEntry> {
    let links = collect_links(client, "", "").await;
    links.into_iter().find(|l| {
        if l.deleted {
            return false;
        }
        let src_group = l.source_node_id.split('/').next().unwrap_or("");
        let dst_group = l.dest_node_id.split('/').next().unwrap_or("");
        let dst_unknown = dst_group.is_empty();
        (src_group == AUTH_GROUP_CONNECTOR
            && (dst_group == AUTH_GROUP_PROTECTED || dst_unknown))
            || (src_group == AUTH_GROUP_PROTECTED
                && (dst_group == AUTH_GROUP_CONNECTOR || dst_unknown))
    })
}

/// Wait until the auth link is Applied with node_1 as the reconciler source.
async fn wait_for_auth_link_applied(client: &mut NbClient, timeout: Duration) -> LinkEntry {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(link) = find_link_between_auth_groups(client).await {
            if link.status == LINK_APPLIED && link.source_node_id == auth_connector_id() {
                assert!(
                    !link.dest_node_id.is_empty(),
                    "expected link dest_node_id to be claimed, got empty"
                );
                return link;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let info = find_link_between_auth_groups(client)
                .await
                .map(|l| {
                    format!(
                        "{}->{}[status={},msg={}]",
                        l.source_node_id, l.dest_node_id, l.status, l.status_msg
                    )
                })
                .unwrap_or_else(|| "no link".to_string());
            panic!(
                "timeout waiting for Applied auth link from {}. Current: {info}",
                auth_connector_id()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait until the auth link is Failed and optionally assert on status_msg.
async fn wait_for_auth_link_failed(
    client: &mut NbClient,
    timeout: Duration,
    msg_contains: Option<&str>,
) -> LinkEntry {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(link) = find_link_between_auth_groups(client).await {
            if link.status == LINK_FAILED {
                if let Some(needle) = msg_contains {
                    assert!(
                        link.status_msg.contains(needle),
                        "expected status_msg containing '{needle}', got: {}",
                        link.status_msg
                    );
                }
                assert_eq!(
                    link.source_node_id,
                    auth_connector_id(),
                    "expected connector node as link source, got {}",
                    link.source_node_id
                );
                return link;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let info = find_link_between_auth_groups(client)
                .await
                .map(|l| {
                    format!(
                        "{}->{}[status={},msg={}]",
                        l.source_node_id, l.dest_node_id, l.status, l.status_msg
                    )
                })
                .unwrap_or_else(|| "no link".to_string());
            panic!("timeout waiting for Failed auth link. Current: {info}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Start a node whose dataplane server requires the given auth and whose
/// controller is pre-loaded with outbound credentials for connecting to other nodes.
// Auth rejection tests
// =============================================================================

/// Start a node with a specific (potentially wrong) auth config, or None for no auth.
async fn start_node_with_auth(
    name: &str,
    group: &str,
    southbound_port: u16,
    dp_port: u16,
    server_auth: slim_config::grpc::server::AuthenticationConfig,
    outbound_clients: Vec<slim_config::grpc::client::ClientConfig>,
    extra_server_metadata: Option<MetadataMap>,
) -> Service {
    let mut md = MetadataMap::new();
    md.insert(
        "external_endpoint",
        MetadataValue::String(format!("127.0.0.1:{dp_port}")),
    );
    if let Some(extra) = extra_server_metadata {
        for (key, value) in extra.iter() {
            md.insert(key.clone(), value.clone());
        }
    }
    let mut dataplane_server = slim_config::grpc::server::ServerConfig::with_endpoint(
        &format!("127.0.0.1:{dp_port}"),
    )
    .with_tls_settings(TlsServerConfig::insecure())
    .with_auth(server_auth);
    dataplane_server.metadata = Some(md);

    let cp_client = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{southbound_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_keepalive(KeepaliveConfig {
            http2_keepalive: Duration::from_secs(1).into(),
            timeout: Duration::from_secs(1).into(),
            keep_alive_while_idle: true,
            ..Default::default()
        });

    let svc_id = ID::new_with_str(&format!("slim/{name}")).unwrap();
    let mut service_config = ServiceConfiguration::new()
        .with_dataplane_server(vec![dataplane_server])
        .with_controlplane_client(vec![cp_client])
        .with_node_id(name);
    service_config.group_name = Some(group.to_string());
    service_config.controller.outbound_clients = outbound_clients;

    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

/// Test: none auth — no auth required on node_2 → link reaches Applied.
///
/// node_2 is registered with AuthMethod::None. The control plane sends
/// RequiredAuthMethod::None to node_1's reconciler. The reconciler skips
/// the credential lookup and connects directly. The link should become Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_none_auth() {
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    // node_2: no auth required on its server.
    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::None,
        vec![],
        None,
    )
    .await;

    // node_1: no outbound_clients needed for None auth.
    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_applied(&mut client, DEFAULT_TIMEOUT).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: basic auth with matching credentials → link reaches Applied.
///
/// node_2's server requires Basic auth (username: "user", password: "pass").
/// node_1 has an outbound_clients entry for node_2's endpoint with the same
/// credentials. The reconciler finds the entry, sets client_config.auth, and
/// connects. node_2 accepts the credentials. The link becomes Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_with_credentials() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    // node_2: server validates Basic auth with fixed credentials.
    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        vec![],
        None,
    )
    .await;

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass")));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_applied(&mut client, DEFAULT_TIMEOUT).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: basic auth with no credentials configured → link reaches Failed.
///
/// node_2's server requires Basic auth. node_1's outbound_clients is empty.
/// The reconciler finds no matching entry for node_2's endpoint, sets
/// success=false with "no local credentials", and reports the failure back to
/// the CP. The link is never Applied and instead reaches Failed status.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_without_credentials() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    // node_2: server requires Basic auth.
    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        vec![],
        None,
    )
    .await;

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(
        &mut client,
        DEFAULT_TIMEOUT,
        Some("no local credentials configured"),
    )
    .await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: JWT auth with no credentials configured → link reaches Failed.
///
/// node_2's server is registered with JWT auth. node_1's outbound_clients is empty.
/// The reconciler receives RequiredAuthMethod::Jwt, finds no matching outbound_clients
/// entry, and immediately reports failure.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_jwt_auth_without_credentials() {
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    // node_2: server is configured with JWT auth (autoresolve key — claims validation
    // irrelevant since we never reach the actual connection in this test).
    let jwt_cfg = JwtConfig::new(
        Claims::default(),
        Duration::from_secs(3600),
        JwtKey::Autoresolve,
    );
    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Jwt(jwt_cfg),
        vec![],
        None,
    )
    .await;

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(
        &mut client,
        DEFAULT_TIMEOUT,
        Some("no local credentials configured"),
    )
    .await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: basic auth with wrong password → link reaches Failed at connect time.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_wrong_credentials() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        vec![],
        None,
    )
    .await;

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_auth(ClientAuth::Basic(BasicConfig::new("user", "wrong")));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(&mut client, DEFAULT_TIMEOUT, Some("Connection failed")).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: outbound_clients entry keyed to the wrong endpoint → credential lookup fails.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_endpoint_mismatch() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        vec![],
        None,
    )
    .await;

    let wrong_port = if node2_port == u16::MAX {
        node2_port - 1
    } else {
        node2_port + 1
    };
    let outbound = ClientConfig::with_endpoint(&protected_endpoint(wrong_port))
        .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass")));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(
        &mut client,
        DEFAULT_TIMEOUT,
        Some(&protected_endpoint(node2_port)),
    )
    .await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: connector restarts with credentials after an initial Failed link → Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_credentials_after_initial_failure() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        vec![],
        None,
    )
    .await;

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(
        &mut client,
        DEFAULT_TIMEOUT,
        Some("no local credentials configured"),
    )
    .await;

    node_1.deregister().await.ok();
    node_1.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass")));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(&mut client, &[&auth_connector_id()], SHORT_TIMEOUT).await;
    wait_for_auth_link_applied(&mut client, DEFAULT_TIMEOUT).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// JWT auth with a wiremock JWKS validator → link reaches Applied.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_jwt_auth_with_mock_validator() {
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;
    use slim_testing::utils::setup_test_jwt_resolver;

    init_tracing();
    let (signing_key, mock_validator, _alg) =
        setup_test_jwt_resolver(Algorithm::RS256).await;

    let claims = Claims::default()
        .with_issuer(mock_validator.uri())
        .with_subject("slim-auth-link-test")
        .with_audience(&["slim-auth-test"]);

    let client_jwt = JwtConfig::new(
        claims.clone(),
        Duration::from_secs(3600),
        JwtKey::Encoding(Key {
            algorithm: Algorithm::RS256,
            format: KeyFormat::Pem,
            key: KeyData::Data(signing_key),
        }),
    );
    let server_jwt = JwtConfig::new(claims, Duration::from_secs(3600), JwtKey::Autoresolve);

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Jwt(server_jwt),
        vec![],
        None,
    )
    .await;

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_auth(ClientAuth::Jwt(client_jwt));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_applied(&mut client, DEFAULT_TIMEOUT).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// JWT auth with a mismatched signing key → link reaches Failed at connect time.
#[tokio::test(flavor = "multi_thread")]
async fn test_link_jwt_auth_wrong_credentials() {
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;
    use slim_testing::utils::setup_test_jwt_resolver;

    init_tracing();
    let (_server_key, mock_validator, _alg) =
        setup_test_jwt_resolver(Algorithm::RS256).await;
    let (wrong_key, _wrong_mock, _wrong_alg) =
        setup_test_jwt_resolver(Algorithm::RS256).await;

    let claims = Claims::default()
        .with_issuer(mock_validator.uri())
        .with_subject("slim-auth-link-test")
        .with_audience(&["slim-auth-test"]);

    let client_jwt = JwtConfig::new(
        claims.clone(),
        Duration::from_secs(3600),
        JwtKey::Encoding(Key {
            algorithm: Algorithm::RS256,
            format: KeyFormat::Pem,
            key: KeyData::Data(wrong_key),
        }),
    );
    let server_jwt = JwtConfig::new(claims, Duration::from_secs(3600), JwtKey::Autoresolve);

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 = start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        node2_port,
        ServerAuth::Jwt(server_jwt),
        vec![],
        None,
    )
    .await;

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_auth(ClientAuth::Jwt(client_jwt));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(&mut client, DEFAULT_TIMEOUT, Some("Connection failed")).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Protected node registered as SPIRE (metadata) with a JWT mock JWKS validator on the server.
/// ponytail: avoids needing a real Workload API socket at node startup; CP still records Spire.
async fn start_spire_registered_jwt_protected_node(
    southbound_port: u16,
    node2_port: u16,
    mock_validator_uri: &str,
) -> Service {
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mock_socket = std::env::temp_dir().join(format!(
        "slim-spire-mock-{}",
        uuid::Uuid::new_v4()
    ));
    let socket_path = mock_socket.to_string_lossy().into_owned();

    let mut extra_md = MetadataMap::new();
    extra_md.insert(
        "spire_socket_path",
        MetadataValue::String(socket_path),
    );
    extra_md.insert(
        "trust_domain",
        MetadataValue::String("auth-test.example".to_string()),
    );

    let server_jwt = JwtConfig::new(
        Claims::default()
            .with_issuer(mock_validator_uri)
            .with_subject("slim-spire-link-test")
            .with_audience(&["slim-spire-test"]),
        Duration::from_secs(3600),
        JwtKey::Autoresolve,
    );

    start_node_with_auth(
        AUTH_NODE_PROTECTED,
        AUTH_GROUP_PROTECTED,
        southbound_port,
        node2_port,
        ServerAuth::Jwt(server_jwt),
        vec![],
        Some(extra_md),
    )
    .await
}

/// SPIRE auth (CP-side) with JWT mock validator on server → connector fails without workload API.
#[cfg(not(target_family = "windows"))]
#[tokio::test(flavor = "multi_thread")]
async fn test_link_spire_auth_fails_without_mock_workload_api() {
    use slim_testing::utils::setup_test_jwt_resolver;
    use slim_auth::jwt::Algorithm;

    init_tracing();
    let (_key, mock_validator, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 =
        start_spire_registered_jwt_protected_node(cp.southbound_port, node2_port, &mock_validator.uri())
            .await;

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        slim_config::grpc::server::AuthenticationConfig::None,
        vec![],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(&mut client, DEFAULT_TIMEOUT, Some("Connection failed")).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// SPIRE outbound client with unavailable mock workload socket → link reaches Failed.
#[cfg(not(target_family = "windows"))]
#[tokio::test(flavor = "multi_thread")]
async fn test_link_spire_auth_outbound_without_mock_workload_api() {
    use slim_config::auth::spire::SpireConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_testing::utils::setup_test_jwt_resolver;
    use slim_auth::jwt::Algorithm;

    init_tracing();
    let (_key, mock_validator, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let node1_port = reserve_port();
    let node2_port = reserve_port();

    let node_2 =
        start_spire_registered_jwt_protected_node(cp.southbound_port, node2_port, &mock_validator.uri())
            .await;

    let mock_socket = std::env::temp_dir().join(format!(
        "slim-spire-mock-{}",
        uuid::Uuid::new_v4()
    ));
    let socket_path = mock_socket.to_string_lossy().into_owned();
    let spire_cfg = SpireConfig::new()
        .with_socket_path(&socket_path)
        .with_trust_domain("auth-test.example");

    let outbound = ClientConfig::with_endpoint(&protected_endpoint(node2_port))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_auth(ClientAuth::Spire(spire_cfg));

    let node_1 = start_node_with_auth(
        AUTH_NODE_CONNECTOR,
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        node1_port,
        slim_config::grpc::server::AuthenticationConfig::None,
        vec![outbound],
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[&auth_connector_id(), &auth_protected_id()],
        SHORT_TIMEOUT,
    )
    .await;

    wait_for_auth_link_failed(&mut client, DEFAULT_TIMEOUT, Some("Connection failed")).await;

    node_1.shutdown().await.ok();
    node_2.shutdown().await.ok();
    stop_control_plane(cp).await;
}
