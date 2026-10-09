// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use agntcy_slim_channel_manager::approval::{APPROVAL_METHOD, APPROVAL_SERVICE};
use agntcy_slim_channel_manager::proto::approval_response::Decision;
use agntcy_slim_channel_manager::proto::channel_manager_service_client::ChannelManagerServiceClient;
use agntcy_slim_channel_manager::proto::channel_manager_service_server::ChannelManagerServiceServer;
use agntcy_slim_channel_manager::proto::{
    AddParticipantRequest, ApprovalRequest, ApprovalResponse, CommandResponse,
    CreateChannelRequest, DeleteChannelRequest, DeleteParticipantRequest, ListChannelsRequest,
    ListParticipantsRequest,
};
use agntcy_slim_channel_manager::service::ChannelManagerServer;
use agntcy_slim_channel_manager::sessions::SessionsList;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use prost::Message;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
use slim_config::client::{
    AuthenticationConfig as ClientAuthenticationConfig, ClientConfig, TransportChannel,
};
use slim_config::component::ComponentBuilder;
use slim_config::grpc::server::{AuthenticationConfig as ServerAuthenticationConfig, ServerConfig};
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::ProtoName;
use slim_rpc::{Context, RpcError, Server};
use slim_service::app::App;
use slim_service::{Service, ServiceBuilder};
use slim_session::{Direction, Notification};
use slim_testing::common::reserve_local_port;

const SHARED_SECRET: &str = "integration-test-shared-secret-0123456789-abcdef";

// --- Helpers ---

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
async fn create_service_and_connect(slim_port: u16, service_name: &str) -> (Arc<Service>, u64) {
    slim_config::tls::provider::initialize_crypto_provider();

    let service = ServiceBuilder::new()
        .build(service_name.to_string())
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

/// Create an app with shared secret authentication that neither sends nor
/// receives data messages, like the channel manager and its participants.
async fn create_app_with_shared_secret(
    service: &Service,
    name: &str,
) -> (
    App<AuthProvider, AuthVerifier>,
    tokio::sync::mpsc::Receiver<Result<slim_session::Notification, slim_session::SessionError>>,
) {
    create_app_with_shared_secret_and_direction(service, name, Direction::None).await
}

/// Create an app with shared secret authentication.
async fn create_app_with_shared_secret_and_direction(
    service: &Service,
    name: &str,
    direction: Direction,
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
        .create_app_with_direction(&app_name, provider, verifier, direction)
        .expect("failed to create app")
}

/// Start the channel-manager gRPC server in-process, plaintext and without
/// auth.
async fn start_channel_manager(
    service: &Arc<Service>,
    conn_id: u64,
    cm_port: u16,
) -> (Arc<App<AuthProvider, AuthVerifier>>, Arc<SessionsList>) {
    let api = ServerConfig::with_endpoint(&format!("127.0.0.1:{cm_port}"))
        .with_tls_settings(TlsServerConfig::insecure());
    start_channel_manager_with_api(service, conn_id, api).await
}

/// Start the channel-manager gRPC server in-process, serving its API with
/// `api` (endpoint, TLS, auth).
async fn start_channel_manager_with_api(
    service: &Arc<Service>,
    conn_id: u64,
    api: ServerConfig,
) -> (Arc<App<AuthProvider, AuthVerifier>>, Arc<SessionsList>) {
    let (app, _rx) = create_app_with_shared_secret(service, "org/ns/channel-manager").await;
    let app = Arc::new(app);

    // Subscribe to the local name
    app.subscribe(app.app_name(), Some(conn_id))
        .await
        .expect("failed to subscribe");

    // Create sessions list and gRPC server
    let sessions = Arc::new(SessionsList::new());
    let server = Arc::new(ChannelManagerServer::new(
        app.clone(),
        conn_id,
        sessions.clone(),
        false,
    ));
    // Fast, so TTL tests don't wait long; channels without a TTL are
    // unaffected. Lives as long as the test's runtime.
    server.spawn_reaper(Duration::from_millis(100));
    let svc = ChannelManagerServiceServer::from_arc(server);

    tokio::spawn(async move {
        let server_future = api
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
    let (app, _rx) = start_receiver_with_notifications(service, local_name, conn_id).await;
    app
}

/// Start a receiver app and keep its notification channel for assertions.
async fn start_receiver_with_notifications(
    service: &Arc<Service>,
    local_name: &str,
    conn_id: u64,
) -> (
    App<AuthProvider, AuthVerifier>,
    tokio::sync::mpsc::Receiver<Result<Notification, slim_session::SessionError>>,
) {
    let (app, rx) = create_app_with_shared_secret(service, local_name).await;
    let app_name = app.app_name().clone();

    // Subscribe to local name
    app.subscribe(&app_name, Some(conn_id))
        .await
        .expect("failed to subscribe receiver");

    (app, rx)
}

/// Create a gRPC client for the channel-manager API.
async fn create_cm_client(cm_port: u16) -> ChannelManagerServiceClient<tonic::transport::Channel> {
    ChannelManagerServiceClient::connect(format!("http://127.0.0.1:{cm_port}"))
        .await
        .expect("failed to connect to channel-manager gRPC API")
}

/// Create a gRPC client for the channel-manager API from a full client
/// config (TLS, auth), the way `cmctl --client-config` does.
async fn create_cm_client_with(
    config: ClientConfig,
) -> ChannelManagerServiceClient<
    impl tonic::client::GrpcService<
        tonic::body::Body,
        Error: Into<tonic::codegen::StdError> + Send,
        ResponseBody: tonic::codegen::Body<
            Data = tonic::codegen::Bytes,
            Error: Into<tonic::codegen::StdError> + Send,
        > + Send
                          + 'static,
        Future: Send,
    > + Send
    + Clone
    + 'static,
> {
    match config
        .to_channel()
        .await
        .expect("failed to create channel-manager gRPC channel")
    {
        TransportChannel::Grpc(channel) => ChannelManagerServiceClient::new(channel),
        TransportChannel::Websocket(_) => panic!("expected a gRPC channel"),
    }
}

/// A throwaway PKI generated when a test starts, so no key material lives in
/// the repository: a CA, and certificates it issues as PEM `(cert, key)`.
struct TestPki {
    ca: CertifiedIssuer<'static, KeyPair>,
}

impl TestPki {
    fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        TestPki { ca }
    }

    fn ca_pem(&self) -> String {
        self.ca.pem()
    }

    fn issue(&self, san: SanType, usage: ExtendedKeyUsagePurpose) -> (String, String) {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.subject_alt_names = vec![san];
        params.extended_key_usages = vec![usage];
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.ca).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    /// A server certificate for 127.0.0.1.
    fn server(&self) -> (String, String) {
        let ip = SanType::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
        self.issue(ip, ExtendedKeyUsagePurpose::ServerAuth)
    }

    /// A client certificate whose URI SAN is `spiffe_id`, as in an
    /// X.509-SVID.
    fn client(&self, spiffe_id: &str) -> (String, String) {
        let uri = SanType::URI(spiffe_id.try_into().unwrap());
        self.issue(uri, ExtendedKeyUsagePurpose::ClientAuth)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_channel_manager_via_cmctl() {
    let slim_port = reserve_local_port();
    let cm_port = reserve_local_port();

    // Start a SLIM node in-process (separate thread with its own runtime).
    let _slim_handle = start_slim_node(slim_port);

    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;

    // Create a service and connect to the SLIM node.
    let (service, conn_id) = create_service_and_connect(slim_port, "test-service").await;

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
            owner_callback_name: None,
            ttl_seconds: None,
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
            grant: None,
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
            grant: None,
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
            owner_callback_name: None,
            ttl_seconds: None,
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
            grant: None,
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
            grant: None,
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
            owner_callback_name: None,
            ttl_seconds: None,
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
            grant: None,
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
            grant: None,
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
            grant: None,
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
            grant: None,
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

#[tokio::test(flavor = "multi_thread")]
async fn test_add_participant_uses_default_gateway_with_separate_services() {
    let slim_port = reserve_local_port();
    let cm_port = reserve_local_port();

    let _slim_handle = start_slim_node(slim_port);
    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;

    // Keep the channel-manager and participant on separate services. This
    // makes the manager's invite depend on its default Edge gateway instead
    // of being satisfied by a local subscription on the same service.
    let (manager_service, manager_conn_id) =
        create_service_and_connect(slim_port, "channel-manager-service").await;
    let (participant_service, participant_conn_id) =
        create_service_and_connect(slim_port, "participant-service").await;

    let (_cm_app, _cm_sessions) =
        start_channel_manager(&manager_service, manager_conn_id, cm_port).await;
    wait_for_port(
        "127.0.0.1",
        cm_port,
        Duration::from_secs(60),
        "channel-manager",
    )
    .await;

    let (participant_app, mut participant_rx) = start_receiver_with_notifications(
        &participant_service,
        "org/ns/default-gateway-participant",
        participant_conn_id,
    )
    .await;

    let mut client = create_cm_client(cm_port).await;

    let response = client
        .create_channel(CreateChannelRequest {
            channel_name: "org/ns/default-gateway-channel".to_string(),
            mls_enabled: true,
            owner_callback_name: None,
            ttl_seconds: None,
        })
        .await
        .expect("create-channel failed")
        .into_inner();
    assert!(
        response.success,
        "create-channel failed: {:?}",
        response.error_msg
    );

    let response = client
        .add_participant(AddParticipantRequest {
            channel_name: "org/ns/default-gateway-channel".to_string(),
            participant_name: "org/ns/default-gateway-participant".to_string(),
            grant: None,
        })
        .await
        .expect("add-participant request failed")
        .into_inner();
    assert!(
        response.success,
        "add-participant failed: {:?}",
        response.error_msg
    );

    let notification = tokio::time::timeout(Duration::from_secs(15), participant_rx.recv())
        .await
        .expect("participant notification timed out")
        .expect("participant notification channel closed")
        .expect("participant received an error");

    match notification {
        Notification::NewSession(session_context) => {
            session_context
                .spawn_receiver(|mut rx, _weak| async move { while rx.recv().await.is_some() {} });
        }
        _ => panic!("expected NewSession notification"),
    }

    drop(participant_app);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_channel_expires_after_its_ttl() {
    let slim_port = reserve_local_port();
    let cm_port = reserve_local_port();

    let _slim_handle = start_slim_node(slim_port);
    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;
    let (service, conn_id) = create_service_and_connect(slim_port, "ttl-service").await;
    let (_cm_app, _cm_sessions) = start_channel_manager(&service, conn_id, cm_port).await;
    wait_for_port(
        "127.0.0.1",
        cm_port,
        Duration::from_secs(60),
        "channel-manager",
    )
    .await;
    let mut client = create_cm_client(cm_port).await;

    let create = |name: &str, ttl_seconds: Option<u64>| CreateChannelRequest {
        channel_name: name.to_string(),
        mls_enabled: false,
        owner_callback_name: None,
        ttl_seconds,
    };

    // A zero TTL is rejected rather than creating an already-expired channel.
    let resp = client
        .create_channel(create("org/ns/zero-ttl", Some(0)))
        .await
        .expect("create-channel request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(resp.error_msg.unwrap().contains("positive"));

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (name, ttl) in [("org/ns/ephemeral", Some(2)), ("org/ns/permanent", None)] {
        let resp = client
            .create_channel(create(name, ttl))
            .await
            .expect("create-channel request failed")
            .into_inner();
        assert!(resp.success, "create {name} failed: {:?}", resp.error_msg);
    }

    // The TTL shows up as an absolute expiry time; no TTL, no expiry.
    let channels = client
        .list_channels(ListChannelsRequest {})
        .await
        .expect("list-channels failed")
        .into_inner()
        .channels;
    let expires_at = |name: &str| {
        channels
            .iter()
            .find(|c| c.channel_name == name)
            .unwrap_or_else(|| panic!("{name} not listed"))
            .expires_at
    };
    let ephemeral_expiry = expires_at("org/ns/ephemeral").expect("ephemeral has no expiry");
    assert!((created_at + 2..=created_at + 3).contains(&ephemeral_expiry));
    assert_eq!(expires_at("org/ns/permanent"), None);

    // Past the TTL (second-granular, plus the reaper's interval), the
    // channel is gone; the one without a TTL is untouched.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let names = client
        .list_channels(ListChannelsRequest {})
        .await
        .expect("list-channels failed")
        .into_inner()
        .channel_name;
    assert!(
        !names.contains(&"org/ns/ephemeral".to_string()),
        "expired channel still listed: {names:?}"
    );
    assert!(names.contains(&"org/ns/permanent".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mtls_client_certificate_identifies_the_caller() {
    let slim_port = reserve_local_port();
    let cm_port = reserve_local_port();

    let _slim_handle = start_slim_node(slim_port);
    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;
    let (service, conn_id) = create_service_and_connect(slim_port, "mtls-service").await;

    // The API requires a client certificate chaining to the test CA, and no
    // token: the caller's only identity is its certificate.
    let pki = TestPki::new();
    let (server_cert, server_key) = pki.server();
    let api = ServerConfig::with_endpoint(&format!("127.0.0.1:{cm_port}")).with_tls_settings(
        TlsServerConfig::new()
            .with_insecure(false)
            .with_cert_and_key_pem(&server_cert, &server_key)
            .with_client_ca_pem(&pki.ca_pem()),
    );
    let (_cm_app, _cm_sessions) = start_channel_manager_with_api(&service, conn_id, api).await;
    wait_for_port(
        "127.0.0.1",
        cm_port,
        Duration::from_secs(60),
        "channel-manager",
    )
    .await;

    let client_as = |spiffe_id: &str| {
        let (cert, key) = pki.client(spiffe_id);
        ClientConfig::with_endpoint(&format!("https://127.0.0.1:{cm_port}")).with_tls_setting(
            TlsClientConfig::new()
                .with_insecure(false)
                .with_ca_pem(&pki.ca_pem())
                .with_cert_and_key_pem(&cert, &key),
        )
    };
    let mut owner = create_cm_client_with(client_as("spiffe://example.org/owner")).await;
    let mut agent = create_cm_client_with(client_as("spiffe://example.org/agent")).await;

    let _participant = start_receiver(&service, "org/ns/mtls-participant", conn_id).await;

    let channel = "org/ns/mtls-channel";
    let resp = owner
        .create_channel(CreateChannelRequest {
            channel_name: channel.to_string(),
            mls_enabled: false,
            owner_callback_name: None,
            ttl_seconds: None,
        })
        .await
        .expect("create-channel request failed")
        .into_inner();
    assert!(resp.success, "create failed: {:?}", resp.error_msg);

    // The creator's certificate SPIFFE ID is recorded as the owner.
    let channels = owner
        .list_channels(ListChannelsRequest {})
        .await
        .expect("list-channels failed")
        .into_inner()
        .channels;
    let listed = channels
        .iter()
        .find(|c| c.channel_name == channel)
        .expect("channel not listed");
    assert_eq!(listed.owner.as_deref(), Some("spiffe://example.org/owner"));

    let add = || AddParticipantRequest {
        channel_name: channel.to_string(),
        participant_name: "org/ns/mtls-participant".to_string(),
        grant: None,
    };

    // A caller with a different certificate isn't the owner: no grant, no
    // change.
    let resp = agent
        .add_participant(add())
        .await
        .expect("add-participant request failed")
        .into_inner();
    assert!(!resp.success);
    assert!(
        resp.error_msg
            .as_deref()
            .unwrap_or_default()
            .contains("requires a grant"),
        "unexpected error: {:?}",
        resp.error_msg
    );

    // The owner needs no grant for their own channel.
    let resp = owner
        .add_participant(add())
        .await
        .expect("add-participant request failed")
        .into_inner();
    assert!(resp.success, "owner add failed: {:?}", resp.error_msg);
}

// --- Owned channels end to end: grants, owner approval, restarts ---

/// HS256 key the channel-manager API verifies caller tokens with.
const API_JWT_SECRET: &str = "integration-test-api-jwt-secret-0123456789";

fn api_jwt_key(encoding: bool) -> JwtKey {
    let key = Key {
        algorithm: Algorithm::HS256,
        format: KeyFormat::Pem,
        key: KeyData::Data(API_JWT_SECRET.to_string()),
    };
    if encoding {
        JwtKey::Encoding(key)
    } else {
        JwtKey::Decoding(key)
    }
}

/// A plaintext channel-manager API that requires a JWT, so every caller has
/// a verified `sub`.
fn jwt_api(cm_port: u16) -> ServerConfig {
    ServerConfig::with_endpoint(&format!("127.0.0.1:{cm_port}"))
        .with_tls_settings(TlsServerConfig::insecure())
        .with_auth(ServerAuthenticationConfig::Jwt(JwtConfig::new(
            Claims::default(),
            Duration::from_secs(3600),
            api_jwt_key(false),
        )))
}

/// Client config for calling a [`jwt_api`] as `subject`.
fn jwt_caller(cm_port: u16, subject: &str) -> ClientConfig {
    ClientConfig::with_endpoint(&format!("http://127.0.0.1:{cm_port}"))
        .with_tls_setting(TlsClientConfig::insecure())
        .with_auth(ClientAuthenticationConfig::Jwt(JwtConfig::new(
            Claims::new(None, None, Some(subject.to_string()), None),
            Duration::from_secs(3600),
            api_jwt_key(true),
        )))
}

/// A channel owner: an Ed25519 key whose `did:key` is the subject they
/// authenticate as, so the default verifier can check grants they sign.
struct Owner {
    key_pair: Ed25519KeyPair,
    did: String,
}

impl Owner {
    fn new() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let mut multicodec = vec![0xed, 0x01];
        multicodec.extend_from_slice(key_pair.public_key().as_ref());
        let did = format!("did:key:z{}", bs58::encode(multicodec).into_string());
        Owner { key_pair, did }
    }

    /// A grant for `action` ("add" or "delete") on `invitee` in `channel`,
    /// valid for an hour. Built from the documented wire format rather than
    /// the crate's internals, so these tests pin that format too.
    fn grant(&self, channel: &str, invitee: &str, action: &str) -> Vec<u8> {
        let not_after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let nonce = uuid::Uuid::new_v4().to_string();
        let signed = [
            "SLIM-CHANNEL-GRANT/1",
            channel,
            invitee,
            action,
            "member",
            &not_after.to_string(),
            &nonce,
        ]
        .join("\0");
        let signature = self.key_pair.sign(signed.as_bytes());
        serde_json::json!({
            "channel": channel,
            "invitee": invitee,
            "action": action,
            "role": "member",
            "not_after": not_after,
            "nonce": nonce,
            "signature": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signature.as_ref(),
            ),
        })
        .to_string()
        .into_bytes()
    }
}

fn create_request(channel: &str, owner_callback_name: Option<&str>) -> CreateChannelRequest {
    CreateChannelRequest {
        channel_name: channel.to_string(),
        mls_enabled: true,
        owner_callback_name: owner_callback_name.map(str::to_string),
        ttl_seconds: None,
    }
}

fn add_request(channel: &str, participant: &str, grant: Option<Vec<u8>>) -> AddParticipantRequest {
    AddParticipantRequest {
        channel_name: channel.to_string(),
        participant_name: participant.to_string(),
        grant,
    }
}

fn assert_refused(resp: &CommandResponse, reason: &str) {
    assert!(!resp.success, "expected a refusal ({reason}), got success");
    let msg = resp.error_msg.as_deref().unwrap_or_default();
    assert!(msg.contains(reason), "expected {reason:?}, got {msg:?}");
}

/// Starts a SLIM node and a service connected to it.
async fn start_node(service_name: &str) -> (u16, std::thread::JoinHandle<()>, Arc<Service>, u64) {
    let slim_port = reserve_local_port();
    let handle = start_slim_node(slim_port);
    wait_for_port("127.0.0.1", slim_port, Duration::from_secs(60), "SLIM node").await;
    let (service, conn_id) = create_service_and_connect(slim_port, service_name).await;
    (slim_port, handle, service, conn_id)
}

/// Starts the channel manager in-process behind a [`jwt_api`].
async fn start_jwt_channel_manager(service: &Arc<Service>, conn_id: u64) -> u16 {
    let cm_port = reserve_local_port();
    let (_app, _sessions) =
        start_channel_manager_with_api(service, conn_id, jwt_api(cm_port)).await;
    wait_for_port(
        "127.0.0.1",
        cm_port,
        Duration::from_secs(60),
        "channel-manager",
    )
    .await;
    cm_port
}

#[tokio::test(flavor = "multi_thread")]
async fn test_grants_gate_participant_changes() {
    let (_slim_port, _node, service, conn_id) = start_node("grants-service").await;
    let cm_port = start_jwt_channel_manager(&service, conn_id).await;

    let owner = Owner::new();
    let mut as_owner = create_cm_client_with(jwt_caller(cm_port, &owner.did)).await;
    let mut as_agent = create_cm_client_with(jwt_caller(cm_port, "org/ns/requester")).await;
    let channel = "org/ns/granted-channel";
    let participant = "org/ns/granted-participant";
    let _participant = start_receiver(&service, participant, conn_id).await;

    let resp = as_owner
        .create_channel(create_request(channel, None))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "create failed: {:?}", resp.error_msg);

    // Not the owner, no grant, and no callback to ask the owner through.
    let resp = as_agent
        .add_participant(add_request(channel, participant, None))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "requires a grant");

    // With the owner's grant the participant really joins.
    let grant = owner.grant(channel, participant, "add");
    let resp = as_agent
        .add_participant(add_request(channel, participant, Some(grant.clone())))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "granted add failed: {:?}", resp.error_msg);
    let members = as_agent
        .list_participants(ListParticipantsRequest {
            channel_name: channel.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .participant_name;
    assert!(
        members.iter().any(|m| m.contains(participant)),
        "{participant} not in {members:?}"
    );

    // The same grant can't be spent twice.
    let resp = as_agent
        .add_participant(add_request(channel, participant, Some(grant)))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "already been used");

    // Removing takes a grant for that action.
    let resp = as_agent
        .delete_participant(DeleteParticipantRequest {
            channel_name: channel.to_string(),
            participant_name: participant.to_string(),
            grant: Some(owner.grant(channel, participant, "delete")),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "granted delete failed: {:?}", resp.error_msg);
}

/// The owner's approval endpoint, in its own service on the node: answers
/// `RequestApproval` with a grant signed by `owner` for participants in
/// `approved` and a denial otherwise, recording each request.
async fn start_owner_endpoint(
    slim_port: u16,
    name: &str,
    owner: Arc<Owner>,
    approved: Vec<String>,
) -> (Arc<Server>, Arc<tokio::sync::Mutex<Vec<ApprovalRequest>>>) {
    let (service, conn_id) = create_service_and_connect(slim_port, "owner-service").await;
    // Answering requests means receiving and sending data.
    let (app, notifications) =
        create_app_with_shared_secret_and_direction(&service, name, Direction::Bidirectional).await;
    let app = Arc::new(app);
    let server = Arc::new(Server::new_with_connection_and_runtime(
        app.clone(),
        app.app_name().clone(),
        Some(conn_id),
        notifications,
        None,
    ));

    let received = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let recorder = received.clone();
    server.register_unary_unary(
        APPROVAL_SERVICE,
        APPROVAL_METHOD,
        move |bytes: Vec<u8>, _ctx: Context| {
            let (owner, approved, recorder) = (owner.clone(), approved.clone(), recorder.clone());
            async move {
                let request = ApprovalRequest::decode(bytes.as_slice())
                    .map_err(|e| RpcError::invalid_argument(e.to_string()))?;
                let decision = if approved.contains(&request.participant_name) {
                    Decision::Grant(owner.grant(
                        &request.channel_name,
                        &request.participant_name,
                        "add",
                    ))
                } else {
                    Decision::Denied("not on the guest list".to_string())
                };
                recorder.lock().await.push(request);
                Ok(ApprovalResponse {
                    decision: Some(decision),
                }
                .encode_to_vec())
            }
        },
    );
    let serving = server.clone();
    tokio::spawn(async move {
        let _ = serving.serve().await;
    });
    // Leaks the owner's service for the test's lifetime, as the node does.
    std::mem::forget(service);
    (server, received)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_owner_approves_and_denies_over_slimrpc() {
    let (slim_port, _node, service, conn_id) = start_node("approval-service").await;
    // The real binary, so the test covers how it wires up owner approval.
    let state_dir = tempfile::tempdir().unwrap();
    let cm_port = reserve_local_port();
    let _process = ChannelManagerProcess::start(slim_port, cm_port, state_dir.path()).await;

    let owner = Arc::new(Owner::new());
    let callback = "org/ns/channel-owner";
    let approved = "org/ns/approved-participant";
    let refused = "org/ns/refused-participant";
    let (_endpoint, received) = start_owner_endpoint(
        slim_port,
        callback,
        owner.clone(),
        vec![approved.to_string()],
    )
    .await;
    let _approved = start_receiver(&service, approved, conn_id).await;
    let _refused = start_receiver(&service, refused, conn_id).await;

    let mut as_owner = create_cm_client_with(jwt_caller(cm_port, &owner.did)).await;
    let mut as_agent = create_cm_client_with(jwt_caller(cm_port, "org/ns/requester")).await;
    let channel = "org/ns/approval-channel";

    let resp = as_owner
        .create_channel(create_request(channel, Some(callback)))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "create failed: {:?}", resp.error_msg);

    // No grant presented: the owner is asked, signs one, and the
    // participant joins.
    let resp = as_agent
        .add_participant(add_request(channel, approved, None))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "approved add failed: {:?}", resp.error_msg);
    let members = as_agent
        .list_participants(ListParticipantsRequest {
            channel_name: channel.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .participant_name;
    assert!(
        members.iter().any(|m| m.contains(approved)),
        "{approved} not in {members:?}"
    );

    let resp = as_agent
        .add_participant(add_request(channel, refused, None))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "not on the guest list");

    // The owner learns who asked, for whom, on which channel.
    let requests = received.lock().await;
    let asked: Vec<_> = requests
        .iter()
        .map(|r| {
            (
                r.channel_name.as_str(),
                r.participant_name.as_str(),
                r.requester.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        asked,
        vec![
            (channel, approved, Some("org/ns/requester")),
            (channel, refused, Some("org/ns/requester")),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_inviting_an_offline_participant_in_api_mode() {
    let (_slim_port, _node, service, conn_id) = start_node("offline-service").await;
    let cm_port = start_jwt_channel_manager(&service, conn_id).await;

    let owner = Owner::new();
    let mut as_owner = create_cm_client_with(jwt_caller(cm_port, &owner.did)).await;
    let channel = "org/ns/offline-channel";
    let resp = as_owner
        .create_channel(create_request(channel, None))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "create failed: {:?}", resp.error_msg);

    // Nobody is subscribed under this name: the invite fails rather than
    // hanging or leaving a phantom member.
    let resp = as_owner
        .add_participant(add_request(channel, "org/ns/nobody-home", None))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "failed to invite participant org/ns/nobody-home");

    let members = as_owner
        .list_participants(ListParticipantsRequest {
            channel_name: channel.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .participant_name;
    assert!(
        !members.iter().any(|m| m.contains("org/ns/nobody-home")),
        "offline participant listed: {members:?}"
    );

    // The channel is still usable: an online participant joins.
    let online = "org/ns/online-participant";
    let _online = start_receiver(&service, online, conn_id).await;
    let resp = as_owner
        .add_participant(add_request(channel, online, None))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "online add failed: {:?}", resp.error_msg);
}

/// The `channel-manager` binary running as a child process; killed on drop.
struct ChannelManagerProcess {
    child: std::process::Child,
    log: std::path::PathBuf,
}

impl ChannelManagerProcess {
    /// Starts the binary with a config for the node at `slim_port`, serving
    /// a [`jwt_api`] on `cm_port` and keeping its state in `state_dir`, and
    /// waits for the API to come up.
    async fn start(slim_port: u16, cm_port: u16, state_dir: &std::path::Path) -> Self {
        let slim_connection = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{slim_port}"))
            .with_tls_setting(TlsClientConfig::insecure());
        let config = serde_json::json!({
            "channel-manager": {
                "slim-connection": slim_connection,
                "api-server": jwt_api(cm_port),
                "local-name": "org/ns/channel-manager",
                "auth": { "type": "shared_secret", "secret": SHARED_SECRET },
                "persistence": {
                    "path": state_dir,
                    "encryption-passphrase": "integration-test-passphrase",
                },
            }
        });
        let config_path = state_dir.join(format!("config-{cm_port}.yaml"));
        std::fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

        let log = state_dir.join(format!("channel-manager-{cm_port}.log"));
        let out = std::fs::File::create(&log).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_channel-manager"))
            .arg("--config-file")
            .arg(&config_path)
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .expect("failed to start channel-manager");
        let process = ChannelManagerProcess { child, log };

        let label = format!("channel-manager (log: {})", process.log.display());
        wait_for_port("127.0.0.1", cm_port, Duration::from_secs(60), &label).await;
        process
    }

    /// Kills the process without letting it shut down cleanly.
    fn crash(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for ChannelManagerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ownership_and_used_grants_survive_a_restart() {
    let (slim_port, _node, service, conn_id) = start_node("restart-service").await;
    let state_dir = tempfile::tempdir().unwrap();

    let owner = Owner::new();
    let channel = "org/ns/durable-channel";
    let first = "org/ns/first-participant";
    let second = "org/ns/second-participant";
    let _first = start_receiver(&service, first, conn_id).await;
    let _second = start_receiver(&service, second, conn_id).await;

    let cm_port = reserve_local_port();
    let process = ChannelManagerProcess::start(slim_port, cm_port, state_dir.path()).await;
    let mut as_owner = create_cm_client_with(jwt_caller(cm_port, &owner.did)).await;
    let mut as_agent = create_cm_client_with(jwt_caller(cm_port, "org/ns/requester")).await;

    let resp = as_owner
        .create_channel(create_request(channel, None))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "create failed: {:?}", resp.error_msg);
    let used = owner.grant(channel, first, "add");
    let resp = as_agent
        .add_participant(add_request(channel, first, Some(used.clone())))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.success, "granted add failed: {:?}", resp.error_msg);

    process.crash();

    // A fresh port, so the restart doesn't race the old listener's teardown.
    let cm_port = reserve_local_port();
    let _process = ChannelManagerProcess::start(slim_port, cm_port, state_dir.path()).await;
    let mut as_agent = create_cm_client_with(jwt_caller(cm_port, "org/ns/requester")).await;

    // The channel came back with its owner.
    let channels = as_agent
        .list_channels(ListChannelsRequest {})
        .await
        .unwrap()
        .into_inner()
        .channels;
    let listed = channels
        .iter()
        .find(|c| c.channel_name == channel)
        .unwrap_or_else(|| panic!("{channel} not restored: {channels:?}"));
    assert_eq!(listed.owner.as_deref(), Some(owner.did.as_str()));

    // Still gated, and the grant used before the restart stays used.
    let resp = as_agent
        .add_participant(add_request(channel, second, None))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "requires a grant");
    let resp = as_agent
        .add_participant(add_request(channel, first, Some(used)))
        .await
        .unwrap()
        .into_inner();
    assert_refused(&resp, "already been used");

    // A new grant still works on the restored channel.
    let resp = as_agent
        .add_participant(add_request(
            channel,
            second,
            Some(owner.grant(channel, second, "add")),
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        resp.success,
        "granted add after restart failed: {:?}",
        resp.error_msg
    );
}
