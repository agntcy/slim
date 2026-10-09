// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::TcpStream;
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

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::client::{ClientConfig, TransportChannel};
use slim_config::component::ComponentBuilder;
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::ProtoName;
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
