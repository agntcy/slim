//! Message routing between two SLIM nodes joined by an external control plane.
//!
//! Both data-plane nodes register with a central `slim-control-plane`, which
//! establishes the inter-node link automatically (topology-based routing). This
//! replaces the removed `slimctl n route add` manual wiring: routes propagate
//! from app subscriptions without any per-node route configuration.

use slim_testing::{
    binaries::{
        require_control_plane_binary, require_sdk_mock_binary, require_slim_binary,
        require_slimctl_binary,
    },
    constants::{MSG_CONNECTED_TO_CONTROL_PLANE, MSG_CONTROL_PLANE_STARTED, MSG_HELLO_FROM_A},
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

struct TwoNodeRoutingSetup {
    cp_north_endpoint: String,
    client_a_config: PathBuf,
    client_b_config: PathBuf,
    control_plane: Option<std::process::Child>,
    server_a: Option<std::process::Child>,
    server_b: Option<std::process::Child>,
    // Owns the temp dir holding the configs above; kept alive for the whole
    // test so the files are not removed until `setup` is dropped.
    _temp_dir: TempDir,
}

impl TwoNodeRoutingSetup {
    fn shutdown(&mut self) {
        terminate_session(&mut self.server_a, Duration::from_secs(30));
        terminate_session(&mut self.server_b, Duration::from_secs(30));
        terminate_session(&mut self.control_plane, Duration::from_secs(30));
    }
}

/// Bring up a control plane plus two SLIM nodes (A and B) registered with it.
fn setup_two_node_routing() -> TwoNodeRoutingSetup {
    let temp_dir = new_temp_dir("slim-integration-routing-");

    let data_plane_a_port = reserve_port();
    let data_plane_b_port = reserve_port();
    let cp_north_port = reserve_port();
    let cp_south_port = reserve_port();

    let server_a_replacements = HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{data_plane_a_port}"),
        ),
        (
            "127.0.0.1:46357".to_string(),
            format!("127.0.0.1:{data_plane_a_port}"),
        ),
        (
            "http://127.0.0.1:50052".to_string(),
            format!("http://127.0.0.1:{cp_south_port}"),
        ),
    ]);
    let server_b_replacements = HashMap::from([
        (
            "0.0.0.0:46367".to_string(),
            format!("0.0.0.0:{data_plane_b_port}"),
        ),
        (
            "127.0.0.1:46367".to_string(),
            format!("127.0.0.1:{data_plane_b_port}"),
        ),
        (
            "http://127.0.0.1:50052".to_string(),
            format!("http://127.0.0.1:{cp_south_port}"),
        ),
    ]);
    let client_a_replacements = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{data_plane_a_port}"),
    )]);
    let client_b_replacements = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{data_plane_b_port}"),
    )]);
    let control_plane_replacements = HashMap::from([
        (
            "0.0.0.0:50051".to_string(),
            format!("0.0.0.0:{cp_north_port}"),
        ),
        (
            "0.0.0.0:50052".to_string(),
            format!("0.0.0.0:{cp_south_port}"),
        ),
    ]);

    let testdata = testdata_dir();
    let server_a_config = write_temp_config(
        temp_dir.path(),
        &testdata.join("routing-server-a.yaml"),
        "routing-server-a.yaml",
        &server_a_replacements,
    );
    let server_b_config = write_temp_config(
        temp_dir.path(),
        &testdata.join("routing-server-b.yaml"),
        "routing-server-b.yaml",
        &server_b_replacements,
    );
    let client_a_config = write_temp_config(
        temp_dir.path(),
        &testdata.join("client.yaml"),
        "client-a-config.yaml",
        &client_a_replacements,
    );
    let client_b_config = write_temp_config(
        temp_dir.path(),
        &testdata.join("client.yaml"),
        "client-b-config.yaml",
        &client_b_replacements,
    );
    let control_plane_config = write_temp_config(
        temp_dir.path(),
        &testdata.join("routing-control-plane.yaml"),
        "routing-control-plane.yaml",
        &control_plane_replacements,
    );
    let db_path = temp_dir.path().join("controlplane.db");
    fs::write(&db_path, []).expect("create control plane db file");

    let slim = require_slim_binary();
    let control_plane = require_control_plane_binary();

    let mut control_plane_session = Some(spawn_control_plane(
        &control_plane,
        &control_plane_config,
        &db_path,
    ));
    let cp_logs = ProcessLogWatcher::attach(
        control_plane_session
            .as_mut()
            .expect("control plane session"),
    );
    cp_logs
        .wait_contains(MSG_CONTROL_PLANE_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            panic!("control plane did not start:\n{output}");
        });

    let mut server_a = Some(spawn_slim(&slim, &server_a_config));
    let mut server_b = Some(spawn_slim(&slim, &server_b_config));
    let server_a_logs = ProcessLogWatcher::attach(server_a.as_mut().expect("server a"));
    let server_b_logs = ProcessLogWatcher::attach(server_b.as_mut().expect("server b"));

    server_a_logs
        .wait_contains(MSG_CONNECTED_TO_CONTROL_PLANE, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            panic!("server A did not connect to control plane:\n{output}");
        });
    server_b_logs
        .wait_contains(MSG_CONNECTED_TO_CONTROL_PLANE, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            panic!("server B did not connect to control plane:\n{output}");
        });

    // Give the control plane a moment to establish the inter-node link.
    thread::sleep(Duration::from_secs(3));

    TwoNodeRoutingSetup {
        cp_north_endpoint: format!("127.0.0.1:{cp_north_port}"),
        client_a_config,
        client_b_config,
        control_plane: control_plane_session,
        server_a,
        server_b,
        _temp_dir: temp_dir,
    }
}

#[test]
fn delivers_messages_both_ways() {
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_two_node_routing();

    // B subscribes first and waits for a message from A (auto-replies).
    let mut client_b = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_b_config,
        "b",
        "a",
        None,
    ));
    let client_b_logs = ProcessLogWatcher::attach(client_b.as_mut().expect("client b"));

    // Give B's subscription time to propagate to A through the control plane.
    thread::sleep(Duration::from_secs(3));

    // A sends to B, then waits for B's auto-reply.
    let mut client_a = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_a_config,
        "a",
        "b",
        Some("hey"),
    ));
    let client_a_logs = ProcessLogWatcher::attach(client_a.as_mut().expect("client a"));

    client_b_logs
        .wait_contains(MSG_HELLO_FROM_A, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            setup.shutdown();
            panic!("client B did not receive message from A:\n{output}");
        });

    client_a_logs
        .wait_contains("hello from the b", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            setup.shutdown();
            panic!("client A did not receive message from B:\n{output}");
        });

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut client_b, Duration::from_secs(2));
    setup.shutdown();
}

#[test]
fn lists_routes_and_connections() {
    let slimctl = require_slimctl_binary();
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_two_node_routing();

    // B subscribes so that a route to org/default/b propagates to node A.
    let mut client_b = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_b_config,
        "b",
        "a",
        None,
    ));
    let _client_b_logs = ProcessLogWatcher::attach(client_b.as_mut().expect("client b"));

    // A sends a message so the delivery path (and thus the route) is exercised.
    let mut client_a = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_a_config,
        "a",
        "b",
        Some("hey"),
    ));
    let _client_a_logs = ProcessLogWatcher::attach(client_a.as_mut().expect("client a"));

    let route_list = run_slimctl_controller_retry(
        &slimctl,
        &setup.cp_north_endpoint,
        &["route", "list", "-n", "domain-a/node-a"],
        Duration::from_secs(15),
    );
    let route_list = String::from_utf8_lossy(&route_list);
    if !route_list.contains("org/default/b") {
        terminate_session(&mut client_a, Duration::from_secs(2));
        terminate_session(&mut client_b, Duration::from_secs(2));
        setup.shutdown();
        panic!("route list on node A should include org/default/b:\n{route_list}");
    }

    let link_list = run_slimctl_controller_retry(
        &slimctl,
        &setup.cp_north_endpoint,
        &["link", "list"],
        Duration::from_secs(15),
    );
    let link_list = String::from_utf8_lossy(&link_list);
    if !(link_list.contains("domain-a") && link_list.contains("domain-b")) {
        terminate_session(&mut client_a, Duration::from_secs(2));
        terminate_session(&mut client_b, Duration::from_secs(2));
        setup.shutdown();
        panic!("link list should include the domain-a<->domain-b link:\n{link_list}");
    }

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut client_b, Duration::from_secs(2));
    setup.shutdown();
}
