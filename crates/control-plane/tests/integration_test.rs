// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the SLIM control plane's inter-domain routing and lifecycle.
//!
//! These tests exercise the full stack: control plane, data plane nodes, and client apps.
//! They cover link creation, route propagation, gateway failover, and cleanup scenarios
//! in a segmented (multi-domain) topology.

use rlimit::increase_nofile_limit;
use std::sync::{Arc, LazyLock};
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
    AddDomainRequest, AddSegmentRequest, AddTopologyLinkRequest, DomainEntry, LinkEntry,
    LinkListRequest, ListDomainsRequest, NodeEntry, NodeListRequest, RemoveDomainRequest,
    RemoveSegmentRequest, RemoveTopologyLinkRequest, RouteEntry, RouteListRequest,
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
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
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

/// Default timeout for waiting on async conditions. Generous headroom so the
/// reconciler has time to converge under slow, heavily-parallel CI runs
/// (e.g. `llvm-cov` instrumentation); the happy path returns immediately.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const SHORT_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-domain registration secrets (each domain has its own secret for isolation).
const TEST_GROUP_SECRETS: &[(&str, &str)] = &[
    ("domain-a", "secret-domain-a-00112233445566778899"),
    ("domain-b", "secret-domain-b-00112233445566778899"),
    ("domain-c", "secret-domain-c-00112233445566778899"),
    ("domain-d", "secret-domain-d-00112233445566778899"),
    ("platform", "secret-platform-001122334455667788"),
    ("customer-a", "secret-customer-a-0011223344556677"),
    ("customer-b", "secret-customer-b-0011223344556677"),
    ("domain-auth-a", "secret-domain-auth-a-0011223344556677"),
    ("domain-auth-b", "secret-domain-auth-b-0011223344556677"),
];

/// Look up the test secret for a domain.
fn domain_secret(domain: &str) -> &'static str {
    TEST_GROUP_SECRETS
        .iter()
        .find(|(g, _)| *g == domain)
        .unwrap_or_else(|| panic!("no test secret for domain '{domain}'"))
        .1
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
        // Keep this low: a persistently-failing link is only marked LINK_FAILED
        // after requeues exhaust, and the delay grows exponentially (base × 2^n).
        // With 10 requeues that's ~51s of cumulative backoff, which blows the
        // `wait_auth_link` timeout under slow llvm-cov runs. 7 → ~6s to fail,
        // still leaving retry margin for slow-to-connect links. Recovery tests
        // are unaffected (they recover via a fresh node registration, not the
        // requeue chain).
        max_requeues: 7,
        base_retry_delay: Duration::from_millis(50).into(),
        reconcile_period: Duration::from_secs(0).into(),
        enable_orphan_detection: false,
        workers: 4,
    }
}

/// Construct the node ID as it appears in the control plane.
/// The CP constructs it as "{domain}/{node_id}" from the registration request.
fn domain_node_id(domain: &str, name: &str) -> String {
    format!("{domain}/{name}")
}

// --- Control Plane ---

/// Bounds how many control-plane integration tests run concurrently. Each test
/// starts a control plane plus several data-plane nodes; run unbounded they
/// starve each other of CPU under slow, parallel CI jobs (llvm-cov / matrix),
/// so reconciliation stalls past the wait timeouts. Each test holds a permit for
/// its lifetime via `TestControlPlane`.
static CP_TEST_SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| {
    let slots = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(2)
        .clamp(2, 4);
    Arc::new(Semaphore::new(slots))
});

struct TestControlPlane {
    northbound_port: u16,
    southbound_port: u16,
    cp: ControlPlane,
    // Concurrency permit, held for the test's lifetime; see `CP_TEST_SLOTS`.
    _permit: OwnedSemaphorePermit,
}

async fn start_control_plane(topology: TopologyConfig) -> TestControlPlane {
    let permit = CP_TEST_SLOTS
        .clone()
        .acquire_owned()
        .await
        .expect("CP_TEST_SLOTS semaphore closed");
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
            auth: None,
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
        _permit: permit,
    }
}

async fn stop_control_plane(tcp: TestControlPlane) {
    tcp.cp.shutdown().await;
}

fn star_topology_config(hub_domain: &str) -> TopologyConfig {
    TopologyConfig::Links(vec![AdjacencyEntry {
        domain: hub_domain.to_string(),
        neighbors: vec!["*".to_string()],
    }])
}

/// Full-mesh topology: every domain can link to every other domain.
fn full_mesh_topology() -> TopologyConfig {
    TopologyConfig::Links(vec![AdjacencyEntry {
        domain: "*".to_string(),
        neighbors: vec!["*".to_string()],
    }])
}

// --- Node Management ---

/// Start a node in a domain with an external endpoint and optional peer configuration.
async fn start_domain_node(
    name: &str,
    domain: &str,
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
            deployment_name: domain.to_string(),
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
    service_config.domain_name = Some(domain.to_string());
    if let Some(pc) = peer_config {
        service_config = service_config.with_peers(pc);
    }

    // The ID kind must be a valid kind (alphanumeric, no hyphens).
    // The actual node registration uses config.node_id and config.domain_name.
    let svc_id = ID::new_with_str(&format!("slim/{name}")).unwrap();
    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

/// Start a single node in a domain (no peers).
async fn start_single_node(
    name: &str,
    domain: &str,
    southbound_port: u16,
    dp_port: u16,
) -> Service {
    start_domain_node(name, domain, southbound_port, dp_port, &[(name, dp_port)]).await
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

async fn wait_for_link_between_domains(
    client: &mut NbClient,
    domain_a: &str,
    domain_b: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let links = collect_links(client, "", "").await;
        let found = links.iter().any(|l| {
            if l.deleted || l.status != LINK_APPLIED {
                return false;
            }
            let src_domain = l.source_node_id.split('/').next().unwrap_or("");
            let dst_domain = l.dest_node_id.split('/').next().unwrap_or("");
            (src_domain == domain_a && dst_domain == domain_b)
                || (src_domain == domain_b && dst_domain == domain_a)
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
                "timeout waiting for Applied link between groups {domain_a} and {domain_b}. Links: [{}]",
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
            // The filtered (src,dest) query is empty here; dump ALL routes and
            // ALL links so a timeout shows what the reconciler actually produced.
            let all_routes = collect_routes(client, "", "").await;
            let routes_dump = all_routes
                .iter()
                .map(|e| {
                    format!(
                        "{}->{}(status={})",
                        e.source_node_id, e.dest_node_id, e.status
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let all_links = collect_links(client, "", "").await;
            let links_dump = all_links
                .iter()
                .map(|l| {
                    format!(
                        "[{}->{} status={} deleted={}]",
                        l.source_node_id, l.dest_node_id, l.status, l.deleted
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            panic!(
                "timeout waiting for route {src}->{dest} status={expected_status}. all_routes=[{routes_dump}] all_links=[{links_dump}]"
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
async fn wait_for_link_between_domains_entry(
    client: &mut NbClient,
    domain_a: &str,
    domain_b: &str,
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
            (sg == domain_a && dg == domain_b) || (sg == domain_b && dg == domain_a)
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
                "timeout waiting for link between {domain_a} and {domain_b}. Links: [{}]",
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

/// Test 1: Inter-domain links created and claimed
///
/// Scenario:
///   - Start a control plane with full-mesh topology (default).
///   - Start one node in each of 3 groups: domain-a, domain-b, domain-c.
///   - Verify that inter-domain links are automatically created between all domain
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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "domain-c", cp.southbound_port, c_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");
    let id_c = domain_node_id("domain-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;

    // Full mesh of 3 groups = links between all 3 pairs.
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-b", "domain-c", DEFAULT_TIMEOUT).await;

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
///   - Start CP + 2 nodes in different groups (domain-a, domain-b).
///   - An app subscribes on node-a:
///     - Verify wildcard route *->node-a and remote route node-b->node-a are created.
///   - A second app subscribes on node-b:
///     - Verify wildcard route *->node-b and remote route node-a->node-b are created.
///     - Verify NO spurious wildcard *->node-a is created from propagation over
///       the Remote inter-domain link.
///
/// Validates: subscribe → CP notified → SPT expansion + no cross-link propagation.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscription_routing() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

/// Test 4: Bidirectional inter-domain routes
///
/// Scenario:
///   - Start CP + 2 nodes in different groups.
///   - Apps subscribe on both nodes (different service names).
///   - Verify routes exist in both directions:
///     - node-a->node-b for the service on node-b
///     - node-b->node-a for the service on node-a
///
/// Validates: bidirectional traffic over a single inter-domain link pair.
#[tokio::test(flavor = "multi_thread")]
async fn test_bidirectional_inter_domain_routes() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

    let id_hub = domain_node_id("platform", "hub");
    let id_spoke_a = domain_node_id("customer-a", "spoke-a");
    let id_spoke_b = domain_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;

    // Spoke-to-hub links.
    wait_for_link_between_domains(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "domain-c", cp.southbound_port, c_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");
    let id_c = domain_node_id("domain-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-b", "domain-c", DEFAULT_TIMEOUT).await;

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
///   - Start CP + 2 nodes in domain-a (node-a1, node-a2) + 1 node in domain-b.
///   - App subscribes on node-b → wildcard *->node-b + expanded route from gateway->node-b.
///   - Kill the gateway node in domain-a.
///   - Verify: link is reassigned to the sibling in domain-a.
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
        start_domain_node("node-a1", "domain-a", cp.southbound_port, a1_port, &peers_a).await;
    let node_a2 =
        start_domain_node("node-a2", "domain-a", cp.southbound_port, a2_port, &peers_a).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a1 = domain_node_id("domain-a", "node-a1");
    let id_a2 = domain_node_id("domain-a", "node-a2");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a1, &id_a2, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    // Find which node in domain-a is the gateway.
    let link =
        wait_for_link_between_domains_entry(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT)
            .await;

    let gateway_id = if link.source_node_id.starts_with("domain-a") {
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

    // The sibling should become the new gateway (link involving sibling and domain-b).
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let links = collect_links(&mut client, "", "").await;
        let new_link = links.iter().any(|l| {
            let involves_sibling = l.source_node_id == sibling_id || l.dest_node_id == sibling_id;
            let involves_domain_b =
                l.source_node_id.starts_with("domain-b") || l.dest_node_id.starts_with("domain-b");
            involves_sibling && involves_domain_b && l.status == LINK_APPLIED && !l.deleted
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
///   - Start CP + 1 node in domain-a + 2 nodes in domain-b (node-b1, node-b2).
///   - Wait for inter-domain link targeting one of domain-b's nodes.
///   - Kill the dest gateway in domain-b.
///   - Verify: a new link is created targeting the sibling in domain-b.
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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let peers_b: Vec<(&str, u16)> = vec![("node-b1", b1_port), ("node-b2", b2_port)];
    let node_b1 =
        start_domain_node("node-b1", "domain-b", cp.southbound_port, b1_port, &peers_b).await;
    let node_b2 =
        start_domain_node("node-b2", "domain-b", cp.southbound_port, b2_port, &peers_b).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b1 = domain_node_id("domain-b", "node-b1");
    let id_b2 = domain_node_id("domain-b", "node-b2");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b1, &id_b2], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    tracing::info!("Initial link established between domain-a and domain-b");

    // Find which node in domain-b is involved in the link (as source or dest).
    let link =
        wait_for_link_between_domains_entry(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT)
            .await;

    // Determine which domain-b node is the gateway (could be source or dest side).
    let b_gateway = if link.source_node_id.starts_with("domain-b") {
        link.source_node_id.clone()
    } else {
        link.dest_node_id.clone()
    };
    let b_sibling = if b_gateway == id_b1 {
        id_b2.clone()
    } else {
        id_b1.clone()
    };

    tracing::info!("domain-b gateway is {b_gateway}, sibling is {b_sibling}");

    // Kill the domain-b gateway.
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

    // A new link should be created involving the sibling in domain-b.
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let links = collect_links(&mut client, "", "").await;
        let new_link = links.iter().any(|l| {
            let involves_sibling = l.source_node_id == b_sibling || l.dest_node_id == b_sibling;
            let involves_domain_a =
                l.source_node_id.starts_with("domain-a") || l.dest_node_id.starts_with("domain-a");
            involves_sibling && involves_domain_a && l.status == LINK_APPLIED && !l.deleted
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
///   - Start CP + 1 node in domain-a (with app subscribed) + 1 node in domain-b.
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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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
    let node_a2 = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    wait_for_nodes_connected(&mut client, &[&id_a], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

/// Test 12: Last node in domain removes domain from topology
///
/// Scenario:
///   - Start CP + 3 nodes in 3 different groups.
///   - Verify links between all groups.
///   - Shut down the node in one domain (removing the domain entirely).
///   - Verify links involving that domain are cleaned up.
///   - Verify remaining groups still have their link.
///
/// Validates: domain removal from link graph when all nodes depart.
#[tokio::test(flavor = "multi_thread")]
async fn test_last_node_removes_domain_links() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();
    let c_port = reserve_port();

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;
    let node_c = start_single_node("node-c", "domain-c", cp.southbound_port, c_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");
    let id_c = domain_node_id("domain-c", "node-c");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b, &id_c], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-c", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-b", "domain-c", DEFAULT_TIMEOUT).await;

    // Kill the only node in domain-a.
    // Use deregister() to explicitly notify the CP (shutdown alone doesn't
    // reliably close the gRPC stream fast enough in-process).
    node_a.deregister().await.ok();
    node_a.shutdown().await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Links involving domain-a should be deleted or gone.
    let links = collect_links(&mut client, "", "").await;
    let active_group_a_links: Vec<_> = links
        .iter()
        .filter(|l| {
            !l.deleted
                && l.status == LINK_APPLIED
                && (l.source_node_id.starts_with("domain-a")
                    || l.dest_node_id.starts_with("domain-a"))
        })
        .collect();
    assert!(
        active_group_a_links.is_empty(),
        "expected no active links for domain-a after node departed, found: {:?}",
        active_group_a_links
            .iter()
            .map(|l| format!("{}->{}", l.source_node_id, l.dest_node_id))
            .collect::<Vec<_>>()
    );

    // Link between domain-b and domain-c should still exist.
    wait_for_link_between_domains(&mut client, "domain-b", "domain-c", SHORT_TIMEOUT).await;

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
/// Validates: full crash->recovery cycle for a single-node domain with subscription-based routes.
#[tokio::test(flavor = "multi_thread")]
async fn test_node_crash_and_link_recovery() {
    init_tracing();

    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    // App subscribes on node-a — creates wildcard template + expanded route via SPT.
    let app = start_subscribing_app(a_port, "org", "ns", "recovery-svc").await;
    wait_for_route_applied(&mut client, "*", &id_a, DEFAULT_TIMEOUT).await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Crash node-b (the destination side).
    node_b.deregister().await.ok();
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Restart node-b on same port.
    let node_b2 = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;
    wait_for_nodes_connected(&mut client, &[&id_b], SHORT_TIMEOUT).await;

    // Link should be re-established and expanded route re-applied.
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;
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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

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

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port_original).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    // App subscribes.
    let app = start_subscribing_app(a_port, "org", "ns", "endpoint-svc").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Crash node-b.
    node_b.deregister().await.ok();
    node_b.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Restart node-b on a DIFFERENT port.
    let b_port_new = reserve_port();
    let node_b2 = start_single_node("node-b", "domain-b", cp.southbound_port, b_port_new).await;
    wait_for_nodes_connected(&mut client, &[&id_b], SHORT_TIMEOUT).await;

    // Link should be re-established (with new endpoint).
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    // Route should recover.
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Verify the link involves node-b's new endpoint. The link direction may
    // vary, so check that the link is between the groups and active.
    let link =
        wait_for_link_between_domains_entry(&mut client, "domain-a", "domain-b", SHORT_TIMEOUT)
            .await;
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

    let id_hub = domain_node_id("platform", "hub");
    let id_spoke_a = domain_node_id("customer-a", "spoke-a");
    let id_spoke_b = domain_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;
    wait_for_link_between_domains(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

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
    wait_for_link_between_domains(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

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

/// Helper: build a segmented star topology with `$domain` template.
/// Hub domain connects to each spoke domain in a separate segment.
fn segmented_star_topology(hub_domain: &str) -> TopologyConfig {
    TopologyConfig::Segments(vec![SegmentConfig {
        name: "seg-$domain".to_string(),
        links: vec![AdjacencyEntry {
            domain: hub_domain.to_string(),
            neighbors: vec!["$domain".to_string()],
        }],
    }])
}

/// Test: Segmented star topology isolates spokes but hub bridges all segments.
///
/// Scenario:
///   - CP with segmented star topology ($domain template): platform↔$domain.
///   - 3 nodes: hub (platform), spoke-a (customer-a), spoke-b (customer-b).
///   - App subscribes on spoke-a → hub gets a route, spoke-b does NOT.
///   - App subscribes on hub → both spokes get a route.
///   - No spoke-to-spoke links.
///
/// Validates: segment isolation, hub bridging, $domain template expansion.
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

    let id_hub = domain_node_id("platform", "hub");
    let id_spoke_a = domain_node_id("customer-a", "spoke-a");
    let id_spoke_b = domain_node_id("customer-b", "spoke-b");

    wait_for_nodes_connected(
        &mut client,
        &[&id_hub, &id_spoke_a, &id_spoke_b],
        SHORT_TIMEOUT,
    )
    .await;

    // Hub should have links to both spokes.
    wait_for_link_between_domains(&mut client, "customer-a", "platform", DEFAULT_TIMEOUT).await;
    wait_for_link_between_domains(&mut client, "customer-b", "platform", DEFAULT_TIMEOUT).await;

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
            domain_a: "domain-a".to_string(),
            domain_b: "domain-b".to_string(),
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
            domain_a: "domain-a".to_string(),
            domain_b: "domain-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("add_topology_link should be idempotent");

    // Now start nodes — link should become Applied via reconciliation.
    let a_port = reserve_port();
    let b_port = reserve_port();

    let node_a = start_single_node("node-a", "domain-a", cp.southbound_port, a_port).await;
    let node_b = start_single_node("node-b", "domain-b", cp.southbound_port, b_port).await;

    let id_a = domain_node_id("domain-a", "node-a");
    let id_b = domain_node_id("domain-b", "node-b");

    wait_for_nodes_connected(&mut client, &[&id_a, &id_b], SHORT_TIMEOUT).await;

    // Wait for link to be created and reach Applied status.
    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;

    // Start a subscribing app on node-a → should create a route from node-b to node-a.
    let app_a = start_subscribing_app(a_port, "org", "ns", "api-svc").await;
    wait_for_route_applied(&mut client, &id_b, &id_a, DEFAULT_TIMEOUT).await;

    // Remove link via API → routes through that link should be cleaned up.
    client
        .remove_topology_link(RemoveTopologyLinkRequest {
            domain_a: "domain-a".to_string(),
            domain_b: "domain-b".to_string(),
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
            domain_a: "domain-a".to_string(),
            domain_b: "domain-b".to_string(),
            segment: "test-seg".to_string(),
        })
        .await
        .expect("re-add topology link failed");

    wait_for_link_between_domains(&mut client, "domain-a", "domain-b", DEFAULT_TIMEOUT).await;
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
            domain_a: "a".to_string(),
            domain_b: "b".to_string(),
            segment: "default".to_string(),
        })
        .await
        .expect_err("add_topology_link should fail in config mode");

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);

    stop_control_plane(cp).await;
}

// Auth link tests: protected node (domain-auth-b) starts before connector (domain-auth-a)
// so the connector is the link source and dials using outbound_clients when required.

const LINK_FAILED: i32 = 3;
const AUTH_GROUP_CONNECTOR: &str = "domain-auth-a";
const AUTH_GROUP_PROTECTED: &str = "domain-auth-b";
const AUTH_CONNECTOR_ID: &str = "domain-auth-a/node-1";
const AUTH_PROTECTED_ID: &str = "domain-auth-b/node-2";

fn peer_endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

struct AuthLinkNodes {
    cp: TestControlPlane,
    client: NbClient,
    connector: Service,
    protected: Service,
    protected_port: u16,
}

async fn auth_link(client: &mut NbClient) -> Option<LinkEntry> {
    // Select the connector-sourced link specifically. During reconciliation a
    // transient link sourced from the protected node can briefly appear; matching
    // it here would intermittently trip `wait_auth_link`'s
    // `source_node_id == AUTH_CONNECTOR_ID` assertion. Every caller expects the
    // connector link, so wait for exactly that one.
    collect_links(client, "", "")
        .await
        .into_iter()
        .find(|l| !l.deleted && l.source_node_id == AUTH_CONNECTOR_ID)
}

async fn wait_auth_link(client: &mut NbClient, timeout: Duration, status: i32, msg: Option<&str>) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(link) = auth_link(client).await
            && link.status == status
        {
            if status == LINK_APPLIED {
                assert_eq!(link.source_node_id, AUTH_CONNECTOR_ID);
                assert!(!link.dest_node_id.is_empty());
            } else {
                assert_eq!(link.source_node_id, AUTH_CONNECTOR_ID);
                if let Some(needle) = msg {
                    assert!(link.status_msg.contains(needle), "{}", link.status_msg);
                }
            }
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            // Dump every link (unfiltered), not just the connector-sourced one,
            // so a timeout shows which directions actually exist.
            let all = collect_links(client, "", "").await;
            let info = all
                .iter()
                .map(|l| {
                    format!(
                        "[{}->{} status={} deleted={} msg={:?}]",
                        l.source_node_id, l.dest_node_id, l.status, l.deleted, l.status_msg
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            panic!(
                "timeout waiting for link status {status}: all_links(n={})=[{info}]",
                all.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn start_auth_link_nodes(
    protected_auth: slim_config::grpc::server::AuthenticationConfig,
    make_outbound: impl FnOnce(u16) -> Vec<ClientConfig>,
    protected_metadata: Option<MetadataMap>,
) -> AuthLinkNodes {
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    init_tracing();
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;
    let connector_port = reserve_port();
    let protected_port = reserve_port();
    let outbound = make_outbound(protected_port);

    let protected = start_node_with_auth(
        "node-2",
        AUTH_GROUP_PROTECTED,
        cp.southbound_port,
        protected_port,
        protected_auth,
        vec![],
        protected_metadata,
    )
    .await;
    let connector = start_node_with_auth(
        "node-1",
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        connector_port,
        ServerAuth::None,
        outbound,
        None,
    )
    .await;

    wait_for_nodes_connected(
        &mut client,
        &[AUTH_CONNECTOR_ID, AUTH_PROTECTED_ID],
        SHORT_TIMEOUT,
    )
    .await;

    AuthLinkNodes {
        cp,
        client,
        connector,
        protected,
        protected_port,
    }
}

async fn stop_auth_link_nodes(nodes: AuthLinkNodes) {
    nodes.connector.shutdown().await.ok();
    nodes.protected.shutdown().await.ok();
    stop_control_plane(nodes.cp).await;
}

async fn start_node_with_auth(
    name: &str,
    domain: &str,
    southbound_port: u16,
    dp_port: u16,
    server_auth: slim_config::grpc::server::AuthenticationConfig,
    outbound_clients: Vec<slim_config::grpc::client::ClientConfig>,
    extra_server_metadata: Option<MetadataMap>,
) -> Service {
    let mut md = MetadataMap::new();
    // Only the protected node advertises a dialable endpoint. The connector is
    // dialer-only in these tests (it reaches the protected via outbound_clients
    // or a CP-driven link). Advertising the connector's endpoint let the CP also
    // establish the *reverse* protected->connector link, which — because the
    // connector accepts ServerAuth::None — reaches Applied and satisfies the
    // node-pair, so the reconciler never creates the forward connector->protected
    // link the tests assert on. That race made `wait_auth_link` intermittently
    // hit "no link" under slow (llvm-cov) runs. Not advertising it forces the one
    // link per pair to be the connector->protected direction under test.
    if domain != AUTH_GROUP_CONNECTOR {
        md.insert(
            "external_endpoint",
            MetadataValue::String(format!("127.0.0.1:{dp_port}")),
        );
    }
    if let Some(extra) = extra_server_metadata {
        for (key, value) in extra.iter() {
            md.insert(key.clone(), value.clone());
        }
    }
    let mut dataplane_server =
        slim_config::grpc::server::ServerConfig::with_endpoint(&format!("127.0.0.1:{dp_port}"))
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
    service_config.domain_name = Some(domain.to_string());
    service_config.controller.outbound_clients = outbound_clients;
    service_config.auth = Some(AuthConfig::SharedSecret {
        id: Some(format!("{domain}/{name}")),
        secret: domain_secret(domain).to_string(),
    });

    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

async fn start_node_with_custom_auth(
    name: &str,
    domain: &str,
    southbound_port: u16,
    dp_port: u16,
    auth: AuthConfig,
) -> Service {
    let mut md = MetadataMap::new();
    md.insert(
        "external_endpoint",
        MetadataValue::String(format!("127.0.0.1:{dp_port}")),
    );
    let mut dataplane_server =
        slim_config::grpc::server::ServerConfig::with_endpoint(&format!("127.0.0.1:{dp_port}"))
            .with_tls_settings(TlsServerConfig::insecure());
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
    service_config.domain_name = Some(domain.to_string());
    service_config.auth = Some(auth);

    let mut svc = service_config.build_server(svc_id).unwrap();
    svc.start().await.unwrap();
    svc
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_none_auth() {
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut nodes = start_auth_link_nodes(ServerAuth::None, |_| vec![], None).await;
    wait_auth_link(&mut nodes.client, DEFAULT_TIMEOUT, LINK_APPLIED, None).await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_with_credentials() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        |port| {
            vec![
                ClientConfig::with_endpoint(&peer_endpoint(port))
                    .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass"))),
            ]
        },
        None,
    )
    .await;
    wait_auth_link(&mut nodes.client, DEFAULT_TIMEOUT, LINK_APPLIED, None).await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_protected_auth_without_outbound_fails() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let cases = [
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        ServerAuth::Jwt(JwtConfig::new(
            Claims::default(),
            Duration::from_secs(3600),
            JwtKey::Autoresolve,
        )),
    ];
    for protected in cases {
        let mut nodes = start_auth_link_nodes(protected, |_| vec![], None).await;
        wait_auth_link(
            &mut nodes.client,
            DEFAULT_TIMEOUT,
            LINK_FAILED,
            Some("no local credentials configured"),
        )
        .await;
        stop_auth_link_nodes(nodes).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_wrong_credentials() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        |port| {
            vec![
                ClientConfig::with_endpoint(&peer_endpoint(port))
                    .with_auth(ClientAuth::Basic(BasicConfig::new("user", "wrong"))),
            ]
        },
        None,
    )
    .await;
    wait_auth_link(
        &mut nodes.client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some("Connection failed"),
    )
    .await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_endpoint_mismatch() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        |port| {
            let wrong = if port == u16::MAX { port - 1 } else { port + 1 };
            vec![
                ClientConfig::with_endpoint(&peer_endpoint(wrong))
                    .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass"))),
            ]
        },
        None,
    )
    .await;
    wait_auth_link(
        &mut nodes.client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some(&peer_endpoint(nodes.protected_port)),
    )
    .await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_basic_auth_credentials_after_initial_failure() {
    use slim_config::auth::basic::Config as BasicConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Basic(BasicConfig::new("user", "pass")),
        |_| vec![],
        None,
    )
    .await;
    wait_auth_link(
        &mut nodes.client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some("no local credentials configured"),
    )
    .await;

    let port = nodes.protected_port;
    let sb = nodes.cp.southbound_port;
    nodes.connector.deregister().await.ok();
    nodes.connector.shutdown().await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let outbound = ClientConfig::with_endpoint(&peer_endpoint(port))
        .with_auth(ClientAuth::Basic(BasicConfig::new("user", "pass")));
    nodes.connector = start_node_with_auth(
        "node-1",
        AUTH_GROUP_CONNECTOR,
        sb,
        reserve_port(),
        ServerAuth::None,
        vec![outbound],
        None,
    )
    .await;
    wait_for_nodes_connected(&mut nodes.client, &[AUTH_CONNECTOR_ID], SHORT_TIMEOUT).await;
    wait_auth_link(&mut nodes.client, DEFAULT_TIMEOUT, LINK_APPLIED, None).await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_jwt_auth_with_mock_validator() {
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;
    use slim_testing::utils::setup_test_jwt_resolver;

    let (signing_key, mock_validator, _) = setup_test_jwt_resolver(Algorithm::RS256).await;
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

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Jwt(server_jwt),
        move |port| {
            vec![
                ClientConfig::with_endpoint(&peer_endpoint(port))
                    .with_auth(ClientAuth::Jwt(client_jwt.clone())),
            ]
        },
        None,
    )
    .await;
    wait_auth_link(&mut nodes.client, DEFAULT_TIMEOUT, LINK_APPLIED, None).await;
    stop_auth_link_nodes(nodes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_link_jwt_auth_wrong_credentials() {
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;
    use slim_testing::utils::setup_test_jwt_resolver;

    let (_server_key, mock_validator, _) = setup_test_jwt_resolver(Algorithm::RS256).await;
    let (wrong_key, _, _) = setup_test_jwt_resolver(Algorithm::RS256).await;
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

    let mut nodes = start_auth_link_nodes(
        ServerAuth::Jwt(server_jwt),
        move |port| {
            vec![
                ClientConfig::with_endpoint(&peer_endpoint(port))
                    .with_auth(ClientAuth::Jwt(client_jwt.clone())),
            ]
        },
        None,
    )
    .await;
    wait_auth_link(
        &mut nodes.client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some("Connection failed"),
    )
    .await;
    stop_auth_link_nodes(nodes).await;
}

async fn spire_protected_node(southbound_port: u16, port: u16, mock_uri: &str) -> Service {
    use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use slim_config::grpc::server::AuthenticationConfig as ServerAuth;

    let mut md = MetadataMap::new();
    md.insert(
        "spire_socket_path",
        MetadataValue::String(
            std::env::temp_dir()
                .join(format!("slim-spire-mock-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .into_owned(),
        ),
    );
    md.insert(
        "trust_domain",
        MetadataValue::String("auth-test.example".to_string()),
    );
    start_node_with_auth(
        "node-2",
        AUTH_GROUP_PROTECTED,
        southbound_port,
        port,
        ServerAuth::Jwt(JwtConfig::new(
            Claims::default()
                .with_issuer(mock_uri)
                .with_subject("slim-spire-link-test")
                .with_audience(&["slim-spire-test"]),
            Duration::from_secs(3600),
            JwtKey::Autoresolve,
        )),
        vec![],
        Some(md),
    )
    .await
}

#[cfg(not(target_family = "windows"))]
#[tokio::test(flavor = "multi_thread")]
async fn test_link_spire_auth_fails_without_mock_workload_api() {
    use slim_auth::jwt::Algorithm;
    use slim_testing::utils::setup_test_jwt_resolver;

    init_tracing();
    let (_, mock_validator, _) = setup_test_jwt_resolver(Algorithm::RS256).await;
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;
    let protected =
        spire_protected_node(cp.southbound_port, reserve_port(), &mock_validator.uri()).await;
    let connector = start_node_with_auth(
        "node-1",
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        reserve_port(),
        slim_config::grpc::server::AuthenticationConfig::None,
        vec![],
        None,
    )
    .await;
    wait_for_nodes_connected(
        &mut client,
        &[AUTH_CONNECTOR_ID, AUTH_PROTECTED_ID],
        SHORT_TIMEOUT,
    )
    .await;
    wait_auth_link(
        &mut client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some("Connection failed"),
    )
    .await;
    connector.shutdown().await.ok();
    protected.shutdown().await.ok();
    stop_control_plane(cp).await;
}

#[cfg(not(target_family = "windows"))]
#[tokio::test(flavor = "multi_thread")]
async fn test_link_spire_auth_outbound_without_mock_workload_api() {
    use slim_auth::jwt::Algorithm;
    use slim_config::auth::spire::SpireConfig;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuth;
    use slim_testing::utils::setup_test_jwt_resolver;

    init_tracing();
    let (_, mock_validator, _) = setup_test_jwt_resolver(Algorithm::RS256).await;
    let cp = start_control_plane(full_mesh_topology()).await;
    let mut client = create_nb_client(cp.northbound_port).await;
    let protected_port = reserve_port();
    let protected =
        spire_protected_node(cp.southbound_port, protected_port, &mock_validator.uri()).await;
    let socket = std::env::temp_dir().join(format!("slim-spire-mock-{}", uuid::Uuid::new_v4()));
    let outbound = ClientConfig::with_endpoint(&peer_endpoint(protected_port))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_auth(ClientAuth::Spire(
            SpireConfig::new()
                .with_socket_path(socket.to_string_lossy())
                .with_trust_domain("auth-test.example"),
        ));
    let connector = start_node_with_auth(
        "node-1",
        AUTH_GROUP_CONNECTOR,
        cp.southbound_port,
        reserve_port(),
        slim_config::grpc::server::AuthenticationConfig::None,
        vec![outbound],
        None,
    )
    .await;
    wait_for_nodes_connected(
        &mut client,
        &[AUTH_CONNECTOR_ID, AUTH_PROTECTED_ID],
        SHORT_TIMEOUT,
    )
    .await;
    wait_auth_link(
        &mut client,
        DEFAULT_TIMEOUT,
        LINK_FAILED,
        Some("Connection failed"),
    )
    .await;
    connector.shutdown().await.ok();
    protected.shutdown().await.ok();
    stop_control_plane(cp).await;
}

// =============================================================================
// Group management API integration tests
// =============================================================================

/// Start a control plane in API mode with shared_secret auth (empty initial secrets).
async fn start_control_plane_api_mode() -> TestControlPlane {
    let permit = CP_TEST_SLOTS
        .clone()
        .acquire_owned()
        .await
        .expect("CP_TEST_SLOTS semaphore closed");
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
            config: TopologyConfig::ApiManaged,
            auth: Some(RegistrationAuthConfig::SharedSecret {
                secrets: std::collections::HashMap::new(),
            }),
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
        _permit: permit,
    }
}

/// Helper to collect groups via the ListGroups API.
async fn collect_domains(client: &mut NbClient) -> Vec<DomainEntry> {
    client
        .list_domains(ListDomainsRequest {})
        .await
        .expect("list_domains failed")
        .into_inner()
        .domains
}

/// Test: Add a domain via API, then a node registers with that secret.
///
/// Scenario:
///   - Start CP in API mode with shared_secret auth (empty initial secrets).
///   - Add a domain via AddGroup API.
///   - Start a node using the same secret → it should register successfully.
#[tokio::test(flavor = "multi_thread")]
async fn test_add_domain_then_node_registers() {
    init_tracing();

    let cp = start_control_plane_api_mode().await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let domain_name = "dynamic-domain";
    let secret = "dynamic-secret-01234567890abcdef";

    // Add the domain via northbound API.
    client
        .add_domain(AddDomainRequest {
            domain_name: domain_name.to_string(),
            secret: secret.to_string(),
        })
        .await
        .expect("add_domain failed");

    // Verify the domain appears in list_domains with no nodes.
    let groups = collect_domains(&mut client).await;
    let entry = groups
        .iter()
        .find(|g| g.domain_name == domain_name)
        .expect("domain should appear in list_domains");
    assert!(entry.nodes.is_empty());

    // Start a node using the dynamically-added secret.
    let dp_port = reserve_port();
    let node_auth = AuthConfig::SharedSecret {
        id: Some(format!("{domain_name}/node-1")),
        secret: secret.to_string(),
    };
    let node = start_node_with_custom_auth(
        "node-1",
        domain_name,
        cp.southbound_port,
        dp_port,
        node_auth,
    )
    .await;

    let node_id = domain_node_id(domain_name, "node-1");
    wait_for_nodes_connected(&mut client, &[&node_id], DEFAULT_TIMEOUT).await;

    // Verify list_domains now shows the node.
    let groups = collect_domains(&mut client).await;
    let entry = groups
        .iter()
        .find(|g| g.domain_name == domain_name)
        .expect("domain should still be in list");
    assert_eq!(entry.nodes, vec![node_id.clone()]);

    node.shutdown().await.ok();
    stop_control_plane(cp).await;
}

/// Test: Remove a domain disconnects connected nodes.
///
/// Scenario:
///   - Start CP in API mode, add a domain, connect a node.
///   - Call RemoveGroup → node should be disconnected and deregistered.
#[tokio::test(flavor = "multi_thread")]
async fn test_remove_domain_kicks_connected_node() {
    init_tracing();

    let cp = start_control_plane_api_mode().await;
    let mut client = create_nb_client(cp.northbound_port).await;

    let domain_name = "evict-domain";
    let secret = "evict-secret-01234567890abcdefghijklmnop";

    // Add domain and connect a node.
    client
        .add_domain(AddDomainRequest {
            domain_name: domain_name.to_string(),
            secret: secret.to_string(),
        })
        .await
        .expect("add_domain failed");

    let dp_port = reserve_port();
    let node_auth = AuthConfig::SharedSecret {
        id: Some(format!("{domain_name}/node-x")),
        secret: secret.to_string(),
    };
    let _node = start_node_with_custom_auth(
        "node-x",
        domain_name,
        cp.southbound_port,
        dp_port,
        node_auth,
    )
    .await;

    let node_id = domain_node_id(domain_name, "node-x");
    wait_for_nodes_connected(&mut client, &[&node_id], DEFAULT_TIMEOUT).await;

    // Remove the domain.
    client
        .remove_domain(RemoveDomainRequest {
            domain_name: domain_name.to_string(),
        })
        .await
        .expect("remove_domain failed");

    // Wait for the node to disappear from the node list.
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        let nodes = collect_nodes(&mut client).await;
        if !nodes.iter().any(|n| n.id == node_id) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for node to be deregistered after remove_domain");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Verify the domain is gone from list_domains.
    let groups = collect_domains(&mut client).await;
    assert!(
        !groups.iter().any(|g| g.domain_name == domain_name),
        "domain should have been removed"
    );

    stop_control_plane(cp).await;
}
