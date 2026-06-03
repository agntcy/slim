// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use agntcy_slim_channel_manager::proto::channel_manager_service_client::ChannelManagerServiceClient;
use agntcy_slim_channel_manager::proto::channel_manager_service_server::ChannelManagerServiceServer;
use agntcy_slim_channel_manager::proto::{
    AddParticipantRequest, CreateChannelRequest, DeleteChannelRequest, DeleteParticipantRequest,
    ListChannelsRequest, ListParticipantsRequest,
};
use agntcy_slim_channel_manager::service::ChannelManagerServer;
use agntcy_slim_channel_manager::sessions::SessionsList;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::client::ClientConfig;
use slim_config::component::ComponentBuilder;
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::ProtoName;
use slim_service::app::App;
use slim_service::{Service, ServiceBuilder};
use slim_session::Direction;

const SHARED_SECRET: &str = "integration-test-shared-secret-0123456789-abcdef";

// --- Helpers ---

fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind local test port");
    let port = listener
        .local_addr()
        .expect("failed to read local address")
        .port();
    drop(listener);
    port
}

async fn wait_for_port(host: &str, port: u16, timeout: Duration, label: &str) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if TcpStream::connect((host, port)).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for {label} on {host}:{port} to accept connections");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Start a SLIM node in-process using the slim crate's config loader and runner.
/// Returns a JoinHandle that completes when the SLIM node exits.
fn start_slim_node(slim_port: u16) -> std::thread::JoinHandle<()> {
    let config_yaml = format!(
        r#"runtime:
  n_cores: 0
  thread_name: "slim-worker"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "127.0.0.1:{slim_port}"
          tls:
            insecure: true
      clients: []
"#
    );

    std::thread::Builder::new()
        .name("slim-test-runtime".to_string())
        .spawn(move || {
            // Write config to a temp file
            use std::io::Write;
            let mut tmp =
                tempfile::NamedTempFile::new().expect("failed to create temp config file");
            tmp.write_all(config_yaml.as_bytes())
                .expect("failed to write temp config");

            let config_path = tmp.path().to_str().unwrap().to_string();
            let mut config = slim::config::ConfigLoader::new(&config_path)
                .expect("failed to load SLIM configuration");

            slim_config::tls::provider::initialize_crypto_provider();

            let runtime =
                slim::runtime::build(config.runtime().expect("invalid runtime configuration"));
            let _ = runtime.block_on(slim::runner::run_services(config));
        })
        .expect("failed to spawn slim runtime thread")
}

/// Create a SLIM service and connect it to the running node.
/// Returns the service and the connection ID.
async fn create_service_and_connect(slim_port: u16) -> (Arc<Service>, u64) {
    slim_config::tls::provider::initialize_crypto_provider();

    let service = ServiceBuilder::new()
        .build("test-service".to_string())
        .expect("failed to build service");
    let service = Arc::new(service);

    let client_config = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{slim_port}"))
        .with_tls_setting(TlsClientConfig::insecure());

    let conn_id = service
        .connect(&client_config)
        .await
        .expect("failed to connect to SLIM node");

    (service, conn_id)
}

/// Create an app with shared secret authentication.
async fn create_app_with_shared_secret(
    service: &Service,
    name: &str,
) -> (
    App<AuthProvider, AuthVerifier>,
    tokio::sync::mpsc::Receiver<Result<slim_session::Notification, slim_session::SessionError>>,
) {
    let app_name = ProtoName::parse_name(name).expect("invalid app name");

    let mut provider =
        AuthProvider::shared_secret_from_str(name, SHARED_SECRET).expect("provider creation");
    let mut verifier =
        AuthVerifier::shared_secret_from_str(name, SHARED_SECRET).expect("verifier creation");

    provider.initialize().await.expect("provider init");
    verifier.initialize().await.expect("verifier init");

    service
        .create_app_with_direction(&app_name, provider, verifier, Direction::None)
        .expect("failed to create app")
}

/// Start the channel-manager gRPC server in-process.
async fn start_channel_manager(
    service: &Arc<Service>,
    conn_id: u64,
    cm_port: u16,
) -> (Arc<App<AuthProvider, AuthVerifier>>, Arc<SessionsList>) {
    let (app, _rx) = create_app_with_shared_secret(service, "org/ns/channel-manager").await;
    let app = Arc::new(app);

    // Subscribe to the local name
    app.subscribe(app.app_name(), Some(conn_id))
        .await
        .expect("failed to subscribe");

    // Create sessions list and gRPC server
    let sessions = Arc::new(SessionsList::new());
    let server = ChannelManagerServer::new(app.clone(), conn_id, sessions.clone());
    let svc = ChannelManagerServiceServer::new(server);

    // Start gRPC server using ServerConfig
    let server_config = ServerConfig::with_endpoint(&format!("127.0.0.1:{cm_port}"))
        .with_tls_settings(TlsServerConfig::insecure());

    tokio::spawn(async move {
        let server_future = server_config
            .to_server_future(&[svc])
            .await
            .expect("failed to create channel-manager gRPC server");
        server_future.await.expect("channel-manager server error");
    });

    (app, sessions)
}

/// Start a receiver app in-process (simulates a channel participant).
async fn start_receiver(
    service: &Arc<Service>,
    local_name: &str,
    conn_id: u64,
) -> App<AuthProvider, AuthVerifier> {
    let (app, _rx) = create_app_with_shared_secret(service, local_name).await;
    let app_name = app.app_name().clone();

    // Subscribe to local name
    app.subscribe(&app_name, Some(conn_id))
        .await
        .expect("failed to subscribe receiver");

    app
}

/// Create a gRPC client for the channel-manager API.
async fn create_cm_client(cm_port: u16) -> ChannelManagerServiceClient<tonic::transport::Channel> {
    ChannelManagerServiceClient::connect(format!("http://127.0.0.1:{cm_port}"))
        .await
        .expect("failed to connect to channel-manager gRPC API")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_channel_manager_via_cmctl() {
    let slim_port = reserve_port();
    let cm_port = reserve_port();

    // Start a SLIM node in-process (separate thread with its own runtime).
    let _slim_handle = start_slim_node(slim_port);

    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;

    // Create a service and connect to the SLIM node.
    let (service, conn_id) = create_service_and_connect(slim_port).await;

    // Start channel-manager in-process.
    let (_cm_app, _cm_sessions) = start_channel_manager(&service, conn_id, cm_port).await;

    wait_for_port(
        "127.0.0.1",
        cm_port,
        Duration::from_secs(60),
        "channel-manager",
    )
    .await;

    let mut client = create_cm_client(cm_port).await;

    // list_channels (empty)
    let resp = client
        .list_channels(ListChannelsRequest {})
        .await
        .expect("list-channels failed")
        .into_inner();
    assert!(resp.success);
    assert_eq!(resp.channel_name.len(), 0);

    // create_channel success
    let resp = client
        .create_channel(CreateChannelRequest {
            channel_name: "org/ns/ch1".to_string(),
            mls_enabled: true,
        })
        .await
        .expect("create-channel failed")
        .into_inner();
    assert!(resp.success, "create-channel failed: {:?}", resp.error_msg);

    // Start 2 receiver apps that act as channel participants.
    let _receiver_1 = start_receiver(&service, "org/ns/p1", conn_id).await;
    let _receiver_2 = start_receiver(&service, "org/ns/p2", conn_id).await;

    // Give receivers time to register
    tokio::time::sleep(Duration::from_secs(10)).await;

    // add_participant success for both running apps
    let resp = client
        .add_participant(AddParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "org/ns/p1".to_string(),
        })
        .await
        .expect("add-participant p1 failed")
        .into_inner();
    assert!(
        resp.success,
        "add-participant p1 failed: {:?}",
        resp.error_msg
    );

    let resp = client
        .add_participant(AddParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "org/ns/p2".to_string(),
        })
        .await
        .expect("add-participant p2 failed")
        .into_inner();
    assert!(
        resp.success,
        "add-participant p2 failed: {:?}",
        resp.error_msg
    );

    // list_participants success contains both participants
    let resp = client
        .list_participants(ListParticipantsRequest {
            channel_name: "org/ns/ch1".to_string(),
        })
        .await
        .expect("list-participants failed")
        .into_inner();
    assert!(resp.success);
    assert!(
        resp.participant_name
            .iter()
            .any(|n| n.contains("org/ns/p1"))
    );
    assert!(
        resp.participant_name
            .iter()
            .any(|n| n.contains("org/ns/p2"))
    );

    // create_channel with invalid name -> error
    let resp = client
        .create_channel(CreateChannelRequest {
            channel_name: "invalid".to_string(),
            mls_enabled: true,
        })
        .await
        .expect("create-channel invalid request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("invalid channel name"),
        "expected 'invalid channel name' error, got: {:?}",
        resp.error_msg
    );

    // add_participant with invalid participant name -> error
    let resp = client
        .add_participant(AddParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "invalid".to_string(),
        })
        .await
        .expect("add-participant invalid request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("invalid participant name"),
        "expected 'invalid participant name' error, got: {:?}",
        resp.error_msg
    );

    // delete_participant with invalid participant name -> error
    let resp = client
        .delete_participant(DeleteParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "invalid".to_string(),
        })
        .await
        .expect("delete-participant invalid request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("invalid participant name"),
        "expected 'invalid participant name' error, got: {:?}",
        resp.error_msg
    );

    // create_channel duplicate -> error
    let resp = client
        .create_channel(CreateChannelRequest {
            channel_name: "org/ns/ch1".to_string(),
            mls_enabled: true,
        })
        .await
        .expect("duplicate create-channel request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("already exists"),
        "expected 'already exists' error, got: {:?}",
        resp.error_msg
    );

    // list_channels includes channel
    let resp = client
        .list_channels(ListChannelsRequest {})
        .await
        .expect("list-channels failed")
        .into_inner();
    assert!(resp.success);
    assert!(resp.channel_name.contains(&"org/ns/ch1".to_string()));

    // list_participants on missing channel -> error
    let resp = client
        .list_participants(ListParticipantsRequest {
            channel_name: "org/ns/missing".to_string(),
        })
        .await
        .expect("list-participants missing request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("not found"),
        "expected 'not found' error, got: {:?}",
        resp.error_msg
    );

    // add_participant on missing channel -> error
    let resp = client
        .add_participant(AddParticipantRequest {
            channel_name: "org/ns/missing".to_string(),
            participant_name: "org/ns/p1".to_string(),
        })
        .await
        .expect("add-participant missing request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("not found"),
        "expected 'not found' error, got: {:?}",
        resp.error_msg
    );

    // delete_participant on missing channel -> error
    let resp = client
        .delete_participant(DeleteParticipantRequest {
            channel_name: "org/ns/missing".to_string(),
            participant_name: "org/ns/p1".to_string(),
        })
        .await
        .expect("delete-participant missing request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("not found"),
        "expected 'not found' error, got: {:?}",
        resp.error_msg
    );

    // delete_participant success for existing channel participants
    let resp = client
        .delete_participant(DeleteParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "org/ns/p1".to_string(),
        })
        .await
        .expect("delete-participant p1 failed")
        .into_inner();
    assert!(
        resp.success,
        "delete-participant p1 failed: {:?}",
        resp.error_msg
    );

    let resp = client
        .delete_participant(DeleteParticipantRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "org/ns/p2".to_string(),
        })
        .await
        .expect("delete-participant p2 failed")
        .into_inner();
    assert!(
        resp.success,
        "delete-participant p2 failed: {:?}",
        resp.error_msg
    );

    // delete_channel on missing channel -> error
    let resp = client
        .delete_channel(DeleteChannelRequest {
            channel_name: "org/ns/missing".to_string(),
        })
        .await
        .expect("delete-channel missing request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or("")
            .contains("not found"),
        "expected 'not found' error, got: {:?}",
        resp.error_msg
    );

    // delete_channel success
    let resp = client
        .delete_channel(DeleteChannelRequest {
            channel_name: "org/ns/ch1".to_string(),
        })
        .await
        .expect("delete-channel failed")
        .into_inner();
    assert!(resp.success, "delete-channel failed: {:?}", resp.error_msg);
}
