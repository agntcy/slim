// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;

// Third-party crates
use display_error_chain::ErrorChainExt;
use tracing::{debug, info};

use slim_auth::traits::TokenProvider;
use slim_config::client::ClientConfig;
use slim_config::component::id::ID;
use slim_datapath::message_processing::MessageProcessor;

// Local crate
use crate::errors::ServiceError;

// Native-only imports
#[cfg(all(not(target_arch = "wasm32"), feature = "kubernetes"))]
use slim_datapath::peer_discovery::KubernetesPeerDiscovery;

#[cfg(not(target_arch = "wasm32"))]
use {
    serde::{Deserialize, Serialize},
    slim_config::component::configuration::Configuration,
    slim_config::component::id::Kind,
    slim_config::component::{Component, ComponentBuilder},
    slim_config::server::ServerConfig,
    slim_controller::config::Config as ControllerConfig,
    slim_controller::config::Config as DataplaneConfig,
    slim_controller::service::ControlPlane,
    slim_datapath::peer_discovery::{PeerConfig, PeerDiscoveryConfig, StaticPeerDiscovery},
    slim_datapath::sync::{PeerSync, PeerSyncConfig},
    slim_datapath::tables::ConnType,
    std::net::SocketAddr,
    std::sync::Arc,
    tokio_util::sync::CancellationToken,
    tracing::warn,
};

// Session feature imports (work on both native and wasm32)
#[cfg(all(feature = "session", not(target_arch = "wasm32")))]
use crate::did_key::{JwkSet, PrivateJwk};
#[cfg(feature = "session")]
use {
    crate::app::{App, bootstrap_app_with_direction},
    slim_auth::traits::Verifier,
    slim_datapath::api::ProtoName,
    slim_session::notification::Notification,
    slim_session::{Direction, SessionError},
    tokio::sync::mpsc,
};

// Define the kind of the component as static string
pub const KIND: &str = "slim";

/// Information about a connection
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Connection ID
    pub id: u64,

    /// Remote address and port (if available)
    pub remote_addr: Option<SocketAddr>,

    /// Local address and port (if available)
    pub local_addr: Option<SocketAddr>,

    /// Endpoint from client configuration (if available)
    pub endpoint: Option<String>,

    /// The connection type (Local, Remote, Peer)
    pub conn_type: ConnType,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Serialize, Deserialize))]
#[cfg_attr(not(target_arch = "wasm32"), serde(rename_all = "snake_case"))]
pub enum DefaultGatewayMode {
    #[default]
    /// The service will use the default gateway.
    Auto,
    /// The service will not use the default gateway.
    Off,
}

// ── ServiceConfiguration ─────────────────────────────────────────────────────
#[derive(Debug, Clone)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[cfg_attr(not(target_arch = "wasm32"), serde(default, deny_unknown_fields))]
pub struct ServiceConfiguration {
    /// Unique node ID for the service. Defaults to a random UUID if not set,
    /// ensuring uniqueness across replicas sharing the same configuration.
    /// This is the **global** identity used for peer sync and controller communication.
    pub node_id: String,

    /// Local service identifier (the YAML map key, e.g. "0" from "slim/0").
    /// Used as the service identifier within this process.
    /// Always set: from the config file key when loading YAML, or defaults to `node_id`.
    #[cfg_attr(not(target_arch = "wasm32"), serde(skip))]
    pub service_id: String,

    /// Optional name of the group for the service.
    pub domain_name: Option<String>,

    /// Optional authentication configuration for control plane registration.
    /// When set, the node will generate and send credentials to prove
    /// group membership during registration.
    #[cfg(not(target_arch = "wasm32"))]
    pub auth: Option<slim_config::auth::AuthConfig>,

    /// DataPlane API configuration
    #[cfg(not(target_arch = "wasm32"))]
    pub dataplane: DataplaneConfig,

    /// Controller API configuration
    #[cfg(not(target_arch = "wasm32"))]
    pub controller: ControllerConfig,

    /// Peer replica configuration for intra-deployment route sync.
    /// When present, enables peer-to-peer subscription synchronization.
    #[cfg(not(target_arch = "wasm32"))]
    pub peers: Option<PeerConfig>,

    #[cfg_attr(not(target_arch = "wasm32"), serde(default))]
    pub default_gateway: DefaultGatewayMode,
}

impl Default for ServiceConfiguration {
    fn default() -> Self {
        let node_id = format!("node-{}", uuid::Uuid::new_v4());
        Self {
            service_id: node_id.clone(),
            node_id,
            domain_name: None,
            #[cfg(not(target_arch = "wasm32"))]
            auth: None,
            #[cfg(not(target_arch = "wasm32"))]
            dataplane: DataplaneConfig::default(),
            #[cfg(not(target_arch = "wasm32"))]
            controller: ControllerConfig::default(),
            #[cfg(not(target_arch = "wasm32"))]
            peers: None,
            default_gateway: DefaultGatewayMode::Auto,
        }
    }
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        ServiceConfiguration::default()
    }

    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = node_id.into();
        self
    }

    /// Returns the service ID (always set — from config key or defaults to node_id).
    pub fn service_id(&self) -> &str {
        &self.service_id
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ServiceConfiguration {
    pub fn with_dataplane_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.dataplane.servers = server;
        self
    }

    pub fn with_dataplane_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.dataplane.clients = clients;
        self
    }

    pub fn dataplane_servers(&self) -> &[ServerConfig] {
        self.dataplane.servers.as_ref()
    }

    pub fn dataplane_clients(&self) -> &[ClientConfig] {
        &self.dataplane.clients
    }

    /// Canonical post-quantum policy for this service (TLS, link negotiation, MLS).
    pub fn enforce_pqc(&self) -> slim_config::EnforcePqcPolicy {
        self.dataplane.enforce_pqc()
    }

    /// Resolve and apply [`EnforcePqcPolicy`] to all dataplane and controlplane TLS endpoints.
    pub fn prepare(&mut self) -> Result<(), ServiceError> {
        self.dataplane.normalize_pqc()?;
        self.controller.normalize_pqc()?;
        Ok(())
    }

    pub fn with_controlplane_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.controller.servers = server;
        self
    }

    pub fn with_controlplane_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.controller.clients = clients;
        self
    }

    pub fn controlplane_servers(&self) -> &[ServerConfig] {
        self.controller.servers.as_ref()
    }

    pub fn controlplane_clients(&self) -> &[ClientConfig] {
        &self.controller.clients
    }

    pub fn with_peers(mut self, peers: PeerConfig) -> Self {
        self.peers = Some(peers);
        self
    }

    pub fn build_server(&self, id: ID) -> Result<Service, ServiceError> {
        let mut config = self.clone();
        config.prepare()?;
        let service = Service::new_with_config(id, config);
        Ok(service)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Configuration for ServiceConfiguration {
    type Error = ServiceError;

    fn validate(&self) -> Result<(), Self::Error> {
        // Validate client and server configurations
        for server in self.dataplane.servers.iter() {
            server.validate()?;
        }
        for client in &self.dataplane.clients {
            client.validate()?;
        }

        // Validate the controller
        self.controller.validate()?;

        Ok(())
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct Service {
    id: ID,

    /// underlying message processor
    message_processor: MessageProcessor,

    /// controller service (native only)
    #[cfg(not(target_arch = "wasm32"))]
    controller: tokio::sync::RwLock<Option<ControlPlane>>,

    /// the configuration of the service
    config: ServiceConfiguration,

    /// cancellation tokens to stop the servers main loop (native only — no servers on wasm32)
    #[cfg(not(target_arch = "wasm32"))]
    cancellation_tokens: parking_lot::RwLock<HashMap<String, CancellationToken>>,

    /// clients created by the service
    #[cfg(not(target_arch = "wasm32"))]
    clients: parking_lot::RwLock<HashMap<String, u64>>,
    // parking_lot is unavailable on wasm32; std::sync::RwLock is safe because
    // wasm32 is single-threaded and will never have concurrent lock attempts.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::disallowed_types)]
    clients: std::sync::RwLock<HashMap<String, u64>>,

    /// Cancellation token for the peer sync manager task (native only).
    #[cfg(not(target_arch = "wasm32"))]
    peer_sync_cancel: parking_lot::Mutex<Option<CancellationToken>>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("Service");
        dbg.field("id", &self.id);
        #[cfg(not(target_arch = "wasm32"))]
        {
            dbg.field("dataplane_servers", &self.config.dataplane_servers());
            dbg.field("dataplane_clients", &self.config.dataplane_clients());
            dbg.field("controller", &self.config.controller);
        }
        dbg.field("domain_name", &self.config.domain_name);
        dbg.finish()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Cancel peer sync manager
            if let Some(token) = self.peer_sync_cancel.lock().take() {
                token.cancel();
            }

            // Trigger all cancellation tokens to stop servers
            for (endpoint, token) in self.cancellation_tokens.write().drain() {
                debug!(%endpoint, "cancelling server on drop");
                token.cancel();
            }
        }

        for (endpoint, conn_id) in self.clients_drain() {
            debug!("disconnecting client on drop: {endpoint} (conn_id={conn_id})");
            if let Err(e) = self.message_processor.disconnect(conn_id) {
                tracing::error!("disconnect error for {endpoint}: {}", e.chain());
            }
        }

        self.message_processor.signal_drain();
    }
}

// ── Private helpers (abstract over parking_lot vs std::sync) ─────────────────

#[allow(clippy::disallowed_types)]
impl Service {
    fn clients_contains_key(&self, key: &str) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.clients.read().contains_key(key)
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clients
                .read()
                .expect("clients lock poisoned")
                .contains_key(key)
        }
    }

    fn clients_get(&self, key: &str) -> Option<u64> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.clients.read().get(key).cloned()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clients
                .read()
                .expect("clients lock poisoned")
                .get(key)
                .cloned()
        }
    }

    fn clients_insert(&self, key: String, value: u64) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.clients.write().insert(key, value);
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clients
                .write()
                .expect("clients lock poisoned")
                .insert(key, value);
        }
    }

    fn clients_remove(&self, key: &str) -> Option<u64> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.clients.write().remove(key)
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clients
                .write()
                .expect("clients lock poisoned")
                .remove(key)
        }
    }

    fn clients_drain(&self) -> Vec<(String, u64)> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.clients.write().drain().collect()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clients
                .write()
                .expect("clients lock poisoned")
                .drain()
                .collect()
        }
    }

    #[cfg(feature = "session")]
    fn enforce_pqc(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.config.enforce_pqc().is_enforced()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
}

// ── Cross-platform Service methods ────────────────────────────────────────────

impl Service {
    /// get the service configuration
    pub fn config(&self) -> &ServiceConfiguration {
        &self.config
    }

    pub async fn shutdown(&self) -> Result<(), ServiceError> {
        debug!("shutting down service");

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Cancel peer sync manager
            if let Some(token) = self.peer_sync_cancel.lock().take() {
                token.cancel();
            }

            // Cancel and drain all server cancellation tokens
            for (endpoint, token) in self.cancellation_tokens.write().drain() {
                info!(%endpoint, "stopping server");
                token.cancel();
            }
        }

        for (endpoint, conn_id) in self.clients_drain() {
            info!("disconnecting client: {endpoint} (conn_id={conn_id})");
            if let Err(e) = self.message_processor.disconnect(conn_id) {
                tracing::error!("disconnect error for {endpoint}: {}", e.chain());
            }
        }

        // Call the shutdown method of message processor to make sure all
        // tasks ended gracefully
        self.message_processor.shutdown().await?;

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref controller) = *self.controller.read().await {
            controller.shutdown().await?;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn connect(&self, config: &ClientConfig) -> Result<u64, ServiceError> {
        if self.clients_contains_key(&config.endpoint) {
            return Err(ServiceError::ClientAlreadyConnected(
                config.endpoint.clone(),
            ));
        }

        let (_handle, conn_id) = self
            .message_processor
            .connect(config.clone(), None, None)
            .await?;

        self.clients_insert(config.endpoint.clone(), conn_id);

        tracing::info!(endpoint = %config.endpoint, conn_id = %conn_id, "client connected");

        Ok(conn_id)
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        let client_config = self.message_processor.disconnect(conn)?;
        let endpoint = client_config.endpoint.clone();

        let stored_conn =
            self.clients_get(&endpoint)
                .ok_or(ServiceError::ConnectionNotFoundForEndpoint(
                    endpoint.clone(),
                ))?;

        if stored_conn == conn {
            self.clients_remove(&endpoint);
            debug!(%endpoint, "removed client mapping");
        } else {
            return Err(ServiceError::DifferentIdForConnection {
                endpoint: endpoint.clone(),
                expected: stored_conn,
                found: conn,
            });
        }

        Ok(())
    }

    pub fn get_connection_id(&self, endpoint: &str) -> Option<u64> {
        self.clients_get(endpoint)
    }

    // APP APIs

    /// Create an app that sends and receives.
    ///
    /// Initializes `identity_provider` and `identity_verifier` first (awaiting their
    /// `initialize`), and fails if either cannot be initialized: the app id is derived
    /// from the provider's token, and a verifier that fetches signing keys (JWT with an
    /// issuer, OIDC, SPIRE) has none until then. Callers must not initialize them
    /// themselves; initializing twice is wasteful for some providers (SPIRE reconnects).
    #[cfg(feature = "session")]
    pub async fn create_app<P, V>(
        &self,
        app_name: &ProtoName,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<
        (
            App<P, V>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    >
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        self.create_app_with_direction(
            app_name,
            identity_provider,
            identity_verifier,
            Direction::Bidirectional,
        )
        .await
    }

    /// Like [`Self::create_app`], with the given traffic direction.
    #[cfg(feature = "session")]
    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn create_app_with_direction<P, V>(
        &self,
        app_name: &ProtoName,
        identity_provider: P,
        identity_verifier: V,
        direction: Direction,
    ) -> Result<
        (
            App<P, V>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    >
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        // Persistence is opt-in; use `create_app_with_direction_and_persistence`
        // to enable it.
        self.create_app_with_direction_and_persistence(
            app_name,
            identity_provider,
            identity_verifier,
            direction,
            None,
        )
        .await
    }

    /// Create an app with an explicit persistence configuration.
    ///
    /// `persistence` enables restorable MLS/session state (encrypted at rest);
    /// `None` disables it. Persistence must be enabled explicitly here — there
    /// is no implicit/environment activation.
    ///
    /// Initializes the provider and the verifier, as [`Self::create_app`] does.
    #[cfg(feature = "session")]
    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn create_app_with_direction_and_persistence<P, V>(
        &self,
        app_name: &ProtoName,
        mut identity_provider: P,
        mut identity_verifier: V,
        direction: Direction,
        persistence: Option<slim_persistence::PersistenceConfig>,
    ) -> Result<
        (
            App<P, V>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    >
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        debug!(%app_name, persistence = persistence.is_some(), "creating app");

        // Before anything derives from them: the app id comes from the provider's token,
        // and verification reads the keys the verifier fetches here.
        identity_provider.initialize().await?;
        identity_verifier.initialize().await?;

        bootstrap_app_with_direction(
            &self.message_processor,
            self.id.to_string(),
            app_name,
            identity_provider,
            identity_verifier,
            direction,
            persistence,
            self.enforce_pqc(),
        )
    }

    /// Create an app whose identity is a `did:key`: it signs with an Ed25519 key and
    /// accepts the peers whose public keys it trusts.
    ///
    /// - `private_key` is the app's private key as a JWK ([`PrivateJwk`]): `kty` `OKP`,
    ///   `crv` `Ed25519` and `d`, the 32-byte seed (RFC 8037). The app's identity is the
    ///   `did:key` of that key: its tokens carry it as issuer and subject.
    /// - `trusted_keys` is the JWKS of the trust anchor: the public keys of the peers the
    ///   app accepts, as `OKP` `Ed25519` JWKs. A token's subject must be the `did:key` of
    ///   the key that signed it. The app's own key is added if the JWKS does not list it:
    ///   MLS verifies the app's own token against the trust anchor.
    ///
    /// The JWKs name their key type, so there is no key type argument: only Ed25519 is
    /// supported. The provider and the verifier are built from them and initialized by
    /// [`Self::create_app`]; the app sends and receives. See
    /// [`Self::create_app_with_did_key_and_direction`] to choose the direction, and
    /// [`did_key_of_private_key`](crate::did_key::did_key_of_private_key) for the
    /// `did:key` of a private key.
    #[cfg(all(feature = "session", not(target_arch = "wasm32")))]
    pub async fn create_app_with_did_key(
        &self,
        app_name: &ProtoName,
        private_key: &PrivateJwk,
        trusted_keys: &JwkSet,
    ) -> Result<
        (
            App<slim_auth::auth_provider::AuthProvider, slim_auth::auth_provider::AuthVerifier>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    > {
        self.create_app_with_did_key_and_direction(
            app_name,
            private_key,
            trusted_keys,
            Direction::Bidirectional,
        )
        .await
    }

    /// Like [`Self::create_app_with_did_key`], with the given traffic direction.
    #[cfg(all(feature = "session", not(target_arch = "wasm32")))]
    pub async fn create_app_with_did_key_and_direction(
        &self,
        app_name: &ProtoName,
        private_key: &PrivateJwk,
        trusted_keys: &JwkSet,
        direction: Direction,
    ) -> Result<
        (
            App<slim_auth::auth_provider::AuthProvider, slim_auth::auth_provider::AuthVerifier>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    > {
        use slim_auth::jwt::KeyData;
        use slim_config::auth::app_auth::AuthConfig;

        let invalid = |e: slim_config::auth::ConfigAuthError| {
            ServiceError::InvalidConfig(format!("invalid did:key identity: {}", e.chain()))
        };
        let private_key_pem = crate::did_key::private_key_pem(private_key)?;
        let own_public_key = crate::did_key::public_key_of(private_key)?;
        let auth = AuthConfig::Jwt {
            private_key: KeyData::Data(private_key_pem),
            trusted_keys: KeyData::Data(crate::did_key::trusted_keys_jwks(
                trusted_keys,
                &own_public_key,
            )?),
        };
        auth.validate().map_err(invalid)?;
        let (provider, verifier) = auth
            .to_identity_configs(&app_name.to_string())
            .map_err(invalid)?;

        self.create_app_with_direction(
            app_name,
            provider.build_auth_provider().map_err(invalid)?,
            verifier.build_auth_verifier().map_err(invalid)?,
            direction,
        )
        .await
    }

    /// Get a reference to the underlying message processor.
    pub fn message_processor(&self) -> &MessageProcessor {
        &self.message_processor
    }
}

// ── Native-only Service constructors and methods ──────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
impl Service {
    pub fn new(id: ID) -> Self {
        Service::new_with_config(id, ServiceConfiguration::new())
    }

    pub fn new_with_config(id: ID, config: ServiceConfiguration) -> Self {
        let deployment_name = config.domain_name.clone().unwrap_or_default();
        let service_id = config.node_id.clone();
        let enforce_pqc = config.enforce_pqc().is_enforced();

        // In full-mesh topology, peers deliver directly (1-hop) so no relay needed.
        // Without peer config (standalone), relay is enabled.
        let relay_peer_publishes = config.peers.is_none();

        let message_processor = if let Some(server) = config.dataplane_servers().first() {
            MessageProcessor::new_with_server_config(
                service_id,
                deployment_name,
                server,
                enforce_pqc,
                relay_peer_publishes,
            )
        } else {
            MessageProcessor::new_with_service_id(service_id, enforce_pqc)
        };
        message_processor.set_default_gateway_enabled(matches!(
            config.default_gateway,
            DefaultGatewayMode::Auto
        ));

        Service {
            id,
            message_processor,
            controller: tokio::sync::RwLock::new(None),
            config,
            cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
            clients: parking_lot::RwLock::new(HashMap::new()),
            peer_sync_cancel: parking_lot::Mutex::new(None),
        }
    }

    pub fn builder() -> ServiceBuilder {
        ServiceBuilder::new()
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn run(&self) -> Result<(), ServiceError> {
        if self.config.dataplane_servers().is_empty() && self.config.dataplane_clients().is_empty()
        {
            return Err(ServiceError::NoServerOrClientConfigured);
        }

        for server in self.config.dataplane_servers().iter() {
            self.run_server(server).await?;
        }

        for client in self.config.dataplane_clients() {
            _ = self.connect(client).await?;
        }

        if let Some(ref peer_config) = self.config.peers.clone() {
            self.start_peer_sync(peer_config);
        }

        if self.config.controller.is_default() {
            info!("no controller configuration provided, skipping controller startup");
            return Ok(());
        }

        debug!("starting controller service");

        let auth_provider = if let Some(auth_config) = &self.config.auth {
            let group = self.config.domain_name.as_ref().ok_or_else(|| {
                ServiceError::InvalidConfig(
                    "domain_name must be set when auth is configured".to_string(),
                )
            })?;
            if self.config.node_id.is_empty() {
                return Err(ServiceError::InvalidConfig(
                    "node_id must be set when auth is configured".to_string(),
                ));
            }
            let identity_name = format!("{}/{}", group, self.config.node_id);
            let registration_auth = auth_config.clone().with_identity_id(identity_name.clone());
            let (provider_config, _) = registration_auth
                .to_identity_configs(&identity_name)
                .map_err(|e| {
                    ServiceError::InvalidConfig(format!("invalid auth configuration: {e}"))
                })?;
            let mut provider = provider_config.build_auth_provider().map_err(|e| {
                ServiceError::InvalidConfig(format!("failed to build auth provider: {e}"))
            })?;
            provider.initialize().await.map_err(|e| {
                ServiceError::InvalidConfig(format!("failed to initialize auth provider: {e}"))
            })?;
            Some(provider)
        } else {
            None
        };

        let mut controller = self.config.controller.into_service(
            self.config.node_id.clone(),
            self.config.domain_name.clone(),
            self.message_processor.clone(),
            self.config.dataplane_servers(),
            self.config.dataplane_clients(),
            auth_provider,
        );

        controller.run().await?;

        *self.controller.write().await = Some(controller);

        Ok(())
    }

    fn start_peer_sync(&self, peer_config: &PeerConfig) {
        let self_id = self.config.node_id.clone();

        let deployment_name = self.config.domain_name.clone().unwrap_or_default();

        info!(
            %self_id,
            topology = ?peer_config.topology,
            %deployment_name,
            "starting peer sync"
        );

        let cancel = CancellationToken::new();
        *self.peer_sync_cancel.lock() = Some(cancel.clone());

        let sync_config = PeerSyncConfig {
            self_id: self_id.clone(),
            deployment_name,
            topology: peer_config.topology.clone(),
        };

        let mp = self.message_processor.clone();
        let peer_sync = PeerSync::with_peer_state(
            &peer_config.topology,
            Arc::new(parking_lot::RwLock::new(
                slim_datapath::sync::PeerState::new(),
            )),
        );
        self.message_processor.set_peer_sync(peer_sync.clone());

        match &peer_config.discovery {
            PeerDiscoveryConfig::Static { peers } => {
                let discovery = StaticPeerDiscovery::from_static_peers(peers, &self_id);
                tokio::spawn(async move {
                    peer_sync
                        .run_discovery(&mp, sync_config, discovery, cancel)
                        .await;
                });
            }
            #[cfg(feature = "kubernetes")]
            PeerDiscoveryConfig::Kubernetes {
                namespace,
                service_name,
                port,
            } => {
                info!(
                    %namespace,
                    %service_name,
                    %port,
                    "starting peer sync (kubernetes EndpointSlice discovery)"
                );
                let discovery = KubernetesPeerDiscovery::new(
                    namespace.clone(),
                    service_name.clone(),
                    *port,
                    self_id,
                );
                tokio::spawn(async move {
                    peer_sync
                        .run_discovery(&mp, sync_config, discovery, cancel)
                        .await;
                });
            }
            #[cfg(not(feature = "kubernetes"))]
            PeerDiscoveryConfig::Kubernetes { .. } => {
                warn!(
                    "kubernetes peer discovery configured but the 'kubernetes' feature is not enabled"
                );
            }
        }
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn deregister(&self) -> Result<(), ServiceError> {
        if let Some(ref controller) = *self.controller.read().await {
            controller.deregister().await?;
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.id))]
    pub async fn run_server(&self, config: &ServerConfig) -> Result<(), ServiceError> {
        let cancellation_token = self.message_processor.run_server(config).await?;
        self.cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), cancellation_token);

        info!(endpoint = %config.endpoint, "dataplane server started");

        Ok(())
    }

    pub fn stop_server(&self, endpoint: &str) -> Result<(), ServiceError> {
        if let Some(token) = self.cancellation_tokens.write().remove(endpoint) {
            token.cancel();
            Ok(())
        } else {
            Err(ServiceError::ServerNotFound(endpoint.to_string()))
        }
    }

    /// Get a list of all connections ordered by connection ID
    pub fn get_all_connections(&self) -> Vec<ConnectionInfo> {
        let clients = self.clients.read();
        let mut connections: Vec<ConnectionInfo> = clients
            .iter()
            .filter_map(|(endpoint, &conn_id)| {
                self.message_processor
                    .connection_table()
                    .get(conn_id)
                    .map(|conn| ConnectionInfo {
                        id: conn_id,
                        remote_addr: conn.remote_addr().copied(),
                        local_addr: conn.local_addr().copied(),
                        endpoint: Some(endpoint.clone()),
                        conn_type: conn.connection_type(),
                    })
            })
            .collect();

        connections.sort_by_key(|c| c.id);
        connections
    }
}

// ── wasm32-only Service constructors ─────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[allow(clippy::disallowed_types)]
impl Service {
    pub fn new(id: ID) -> Self {
        Service::new_with_config(id, ServiceConfiguration::new())
    }

    pub fn new_with_config(id: ID, config: ServiceConfiguration) -> Self {
        let service_id = config.node_id.clone();
        let message_processor = MessageProcessor::new_with_service_id(service_id, false);
        message_processor.set_default_gateway_enabled(matches!(
            config.default_gateway,
            DefaultGatewayMode::Auto
        ));
        Service {
            id,
            message_processor,
            config,
            clients: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

// ── Component / ServiceBuilder (native only) ──────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl Component for Service {
    type Error = ServiceError;

    fn identifier(&self) -> &ID {
        &self.id
    }

    async fn start(&mut self) -> Result<(), Self::Error> {
        debug!("starting service");
        let res = self.run().await?;

        Ok(res)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(PartialEq, Eq, Hash, Default)]
pub struct ServiceBuilder;

#[cfg(not(target_arch = "wasm32"))]
impl ServiceBuilder {
    pub fn new() -> Self {
        ServiceBuilder {}
    }

    pub fn kind() -> Kind {
        Kind::new(KIND).unwrap()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ComponentBuilder for ServiceBuilder {
    type Config = ServiceConfiguration;
    type Component = Service;

    fn kind(&self) -> Kind {
        ServiceBuilder::kind()
    }

    fn build(&self, name: String) -> Result<Self::Component, ServiceError> {
        let id = ID::new_with_name(ServiceBuilder::kind(), name.as_ref())?;

        Ok(Service::new(id))
    }

    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, ServiceError> {
        let id = ID::new_with_name(ServiceBuilder::kind(), name)?;
        config.build_server(id)
    }
}

// ── Tests (native only) ───────────────────────────────────────────────────────

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_config::client::ClientConfig;
    use slim_config::server::ServerConfig;
    use slim_config::tls::server::TlsServerConfig;
    use slim_datapath::api::MessageType;
    use slim_datapath::api::ProtoName;
    use slim_datapath::peer_discovery::{
        PeerConfig, PeerDiscoveryConfig, PeerTopology, StaticPeerEntry,
    };
    use slim_session::SessionConfig;
    use slim_session::session_config::MlsSettings;
    use slim_testing::utils::TEST_VALID_SECRET;
    use std::time::Duration;
    use tokio::time;
    use tracing_test::traced_test;

    fn jwt_verifier_for(issuer: &str) -> slim_auth::jwt::VerifierJwt {
        slim_auth::builder::JwtBuilder::new()
            .issuer(issuer)
            .audience(&["test-audience"])
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build()
            .unwrap()
    }

    /// `create_app` initializes the verifier: the signing keys of its issuer are fetched
    /// by the time it returns, so the first token verifies without waiting.
    #[tokio::test]
    async fn test_create_app_initializes_the_verifier() {
        use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
        use slim_auth::traits::{Signer, StandardClaims, Verifier};

        slim_config::tls::provider::initialize_crypto_provider();
        let (signing_key, issuer, _) =
            slim_testing::utils::setup_test_jwt_resolver(Algorithm::RS256).await;
        let signer = slim_auth::builder::JwtBuilder::new()
            .issuer(issuer.uri())
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::RS256,
                format: KeyFormat::Pem,
                key: KeyData::Data(signing_key),
            })
            .build()
            .unwrap();
        let token = signer.sign(&signer.create_claims()).unwrap();

        let verifier = jwt_verifier_for(&issuer.uri());
        // Not initialized yet: it has no keys, and the issuer was not asked.
        assert!(
            Verifier::get_claims::<StandardClaims>(&verifier, &token)
                .is_err_and(|e| matches!(e, slim_auth::errors::AuthError::WouldBlockOn))
        );

        let service = Service::builder().build("svc-init".to_string()).unwrap();
        let name = ProtoName::from_strings(["org", "ns", "app"]);
        let provider = SharedSecret::new("app", TEST_VALID_SECRET).unwrap();
        let _app = service
            .create_app(&name, provider, verifier.clone())
            .await
            .unwrap();

        // The clone the app got shares the keys that initialization fetched.
        let claims: StandardClaims = Verifier::get_claims(&verifier, &token).unwrap();
        assert_eq!(claims.sub.unwrap(), "test-subject");
    }

    /// An identity that cannot be initialized fails `create_app`, instead of an app that
    /// cannot verify anyone.
    #[tokio::test]
    async fn test_create_app_fails_when_the_verifier_cannot_initialize() {
        slim_config::tls::provider::initialize_crypto_provider();
        let service = Service::builder()
            .build("svc-init-fail".to_string())
            .unwrap();
        let name = ProtoName::from_strings(["org", "ns", "app"]);
        let provider = SharedSecret::new("app", TEST_VALID_SECRET).unwrap();

        let result = service
            .create_app(&name, provider, jwt_verifier_for("http://127.0.0.1:1"))
            .await;
        assert!(matches!(result, Err(ServiceError::ProviderAuthError(_))));
    }

    // RFC 8037 appendix A: the key of RFC 8032 section 7.1, test 1, and the public key of
    // test 2.
    const SEED_1: &str = "nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A";
    const PUBLIC_1: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
    const PUBLIC_2: &str = "PUAXw-hDiVqStwqnTRt-vJyYLM8uxJaMwM1V8Sr0Zgw";

    fn private_jwk(d: &str) -> PrivateJwk {
        PrivateJwk::from_json(&format!(r#"{{"kty":"OKP","crv":"Ed25519","d":"{d}"}}"#)).unwrap()
    }

    fn jwks_of(public_keys: &[&str]) -> JwkSet {
        let keys: Vec<String> = public_keys
            .iter()
            .map(|x| {
                format!(r#"{{"kty":"OKP","crv":"Ed25519","alg":"EdDSA","use":"sig","x":"{x}"}}"#)
            })
            .collect();
        serde_json::from_str(&format!(r#"{{"keys":[{}]}}"#, keys.join(","))).unwrap()
    }

    /// The helper converts a private JWK and a JWKS, and creates the app.
    #[tokio::test]
    async fn test_create_app_with_did_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let service = Service::builder().build("svc-did-key".to_string()).unwrap();
        let name = ProtoName::from_strings(["org", "ns", "did-app"]);

        let (app, _rx) = service
            .create_app_with_did_key(&name, &private_jwk(SEED_1), &jwks_of(&[PUBLIC_2]))
            .await
            .expect("an app with a did:key identity");

        // The app id is derived from the identity: the did:key of the private key.
        let did = crate::did_key::did_key_of_private_key(&private_jwk(SEED_1)).unwrap();
        assert_eq!(app.app_name().id(), slim_session::name_id::app_id(&did));
    }

    /// An app that trusts nobody else still trusts its own key, which MLS needs.
    #[tokio::test]
    async fn test_create_app_with_did_key_trusts_its_own_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let service = Service::builder()
            .build("svc-did-key-2".to_string())
            .unwrap();

        service
            .create_app_with_did_key(
                &ProtoName::from_strings(["org", "ns", "did-app"]),
                &private_jwk(SEED_1),
                &jwks_of(&[]),
            )
            .await
            .expect("an app that trusts only itself");
        // Listing the own key too is not a problem.
        service
            .create_app_with_did_key(
                &ProtoName::from_strings(["org", "ns", "did-app-2"]),
                &private_jwk(SEED_1),
                &jwks_of(&[PUBLIC_1]),
            )
            .await
            .expect("an app whose trust anchor lists its own key");
    }

    /// Keys that are not Ed25519 JWKs are an invalid configuration, naming what was
    /// expected.
    #[tokio::test]
    async fn test_create_app_with_did_key_rejects_other_key_types() {
        let service = Service::builder()
            .build("svc-did-key-3".to_string())
            .unwrap();
        let name = ProtoName::from_strings(["org", "ns", "did-app"]);

        let p256 = PrivateJwk::from_json(r#"{"kty":"EC","crv":"P-256","d":"AAAA"}"#).unwrap();
        let result = service
            .create_app_with_did_key(&name, &p256, &jwks_of(&[]))
            .await;
        let Err(ServiceError::InvalidConfig(message)) = result else {
            panic!("expected an invalid configuration, got {:?}", result.err());
        };
        assert!(message.contains("only OKP Ed25519"), "{message}");

        let rsa: JwkSet = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","n":"sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw","e":"AQAB"}]}"#,
        )
        .unwrap();
        let result = service
            .create_app_with_did_key(&name, &private_jwk(SEED_1), &rsa)
            .await;
        let Err(ServiceError::InvalidConfig(message)) = result else {
            panic!("expected an invalid configuration, got {:?}", result.err());
        };
        assert!(message.contains("not an OKP Ed25519 JWK"), "{message}");
    }

    #[tokio::test]
    async fn test_service_configuration() {
        let config = ServiceConfiguration::new();
        assert_eq!(config.dataplane_servers(), &[]);
        assert_eq!(config.dataplane_clients(), &[]);
    }

    #[test]
    fn test_build_server_uses_passed_id() {
        let mut config = ServiceConfiguration::new();
        config.node_id = "custom-node".to_string();

        let original_id = ID::new_with_name(Kind::new(KIND).unwrap(), "original").unwrap();
        let service = config.build_server(original_id).unwrap();

        // build_server uses the ID passed directly
        assert_eq!(service.identifier().name(), "original");
        assert_eq!(service.identifier().kind(), &Kind::new(KIND).unwrap());
    }

    #[test]
    fn test_build_with_config_uses_node_id() {
        let mut config = ServiceConfiguration::new();
        config.node_id = "custom-node".to_string();

        let builder = ServiceBuilder::new();
        let service = builder.build_with_config("custom-node", &config).unwrap();

        // build_with_config uses the name parameter (which is node_id from config)
        assert_eq!(service.identifier().name(), "custom-node");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_skips_controller_when_config_is_default() {
        // Create a service with only dataplane server config, no controller config
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12347").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(
                ID::new_with_name(Kind::new(KIND).unwrap(), "test-no-controller").unwrap(),
            )
            .unwrap();

        // Run the service - should start dataplane but skip controller
        service.run().await.expect("failed to run service");

        // Wait a bit for logs to be generated
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify controller was skipped
        assert!(logs_contain(
            "no controller configuration provided, skipping controller startup"
        ));
        // Verify dataplane still started
        assert!(logs_contain("dataplane server started"));

        // Graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_starts_controller_when_config_is_provided() {
        // Create a service with both dataplane and controller configurations
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let dataplane_server_config =
            ServerConfig::with_endpoint("0.0.0.0:12348").with_tls_settings(tls_config.clone());
        let controller_server_config =
            ServerConfig::with_endpoint("0.0.0.0:12349").with_tls_settings(tls_config);

        let config = ServiceConfiguration::new()
            .with_dataplane_server(vec![dataplane_server_config])
            .with_controlplane_server(vec![controller_server_config]);

        let service = config
            .build_server(
                ID::new_with_name(Kind::new(KIND).unwrap(), "test-with-controller").unwrap(),
            )
            .unwrap();

        // Run the service - should start both dataplane and controller
        service.run().await.expect("failed to run service");

        // Wait a bit for logs to be generated
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify controller was started (not skipped)
        assert!(!logs_contain(
            "no controller configuration provided, skipping controller startup"
        ));
        assert!(logs_contain("starting controller service"));
        // Verify dataplane also started
        assert!(logs_contain("dataplane server started"));

        // Graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_build_server() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        service.run().await.expect("failed to run service");

        // wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // assert that the service is running
        assert!(logs_contain("dataplane server started"));

        // graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");

        assert!(logs_contain("shutting down service"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_disconnection() {
        // create the service (server + one client we will disconnect)
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12346").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test-disconnect").unwrap())
            .unwrap();

        service.run().await.expect("failed to run service");

        // wait a bit for server loop to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // build client configuration and connect
        let mut client_conf =
            slim_config::client::ClientConfig::with_endpoint("http://0.0.0.0:12346");
        client_conf.tls_setting.insecure = true;
        let conn_id = service
            .connect(&client_conf)
            .await
            .expect("failed to connect client");

        assert!(service.get_connection_id(&client_conf.endpoint).is_some());

        // disconnect
        service
            .disconnect(conn_id)
            .expect("disconnect should succeed");

        // allow cancellation token to propagate and stream to terminate
        tokio::time::sleep(Duration::from_millis(200)).await;

        // verify connection is removed from internal client mapping
        assert!(
            service.get_connection_id(&client_conf.endpoint).is_none(),
            "client mapping should be removed after disconnect"
        );

        // verify connection is removed from connection table
        assert!(
            service
                .message_processor
                .connection_table()
                .get(conn_id)
                .is_none(),
            "connection should be removed after disconnect"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_publish_subscribe() {
        // in this test, we create a publisher and a subscriber and test the
        // communication between them

        info!("starting test_service_publish_subscribe");

        // create the service
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // create a subscriber
        let subscriber_name =
            ProtoName::from_strings(["cisco", "default", "subscriber"]).with_id(0);
        let (sub_app, mut sub_rx) = service
            .create_app(
                &subscriber_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .await
            .expect("failed to create app");

        // create a publisher
        let publisher_name = ProtoName::from_strings(["cisco", "default", "publisher"]).with_id(0);
        let (pub_app, _rx) = service
            .create_app(
                &publisher_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .await
            .expect("failed to create app");

        // sleep to allow the subscription to be processed
        time::sleep(Duration::from_millis(100)).await;

        // NOTE: here we don't call any subscribe as the publisher and the subscriber
        // are in the same service (so they share one single slim instance) and the
        // subscription is done automatically.

        // create a point to point session
        let mut config = SessionConfig::default()
            .with_session_type(slim_datapath::api::ProtoSessionType::PointToPoint);
        config.initiator = true;
        let mut dest = subscriber_name.clone();
        dest.reset_id();
        let (send_session, completion_handle) =
            pub_app.create_session(config, dest, None).await.unwrap();

        completion_handle.await.expect("session creation failed");

        // publish a message
        let message_blob = "very complicated message".as_bytes().to_vec();
        send_session
            .session_arc()
            .unwrap()
            .publish(&subscriber_name, message_blob.clone(), None, None)
            .await
            .unwrap();

        // wait for the new session to arrive in the subscriber app
        // and check the message is correct
        let session = sub_rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let mut recv_session = match session {
            Notification::NewSession(s) => s,
            _ => panic!("expected a point to point session"),
        };

        // Let's receive now the message from the session
        let msg = recv_session
            .rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // make sure message is a publication
        assert!(msg.message_type.is_some());

        // make sure the session ids correspond
        assert_eq!(
            send_session.session_arc().unwrap().id(),
            msg.get_session_header().get_session_id()
        );

        let publ = match msg.message_type.unwrap() {
            MessageType::Publish(p) => p,
            _ => panic!("expected a publication"),
        };

        // make sure message is correct
        assert_eq!(
            publ.get_payload().as_application_payload().unwrap().blob,
            message_blob
        );

        // Now remove the session from the 2 apps
        pub_app
            .delete_session(&send_session.session_arc().unwrap())
            .unwrap();
        sub_app
            .delete_session(&recv_session.session_arc().unwrap())
            .unwrap();

        // And drop the 2 apps
        drop(pub_app);
        drop(sub_app);

        // sleep to allow the deletion to be processed
        time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_session_configuration() {
        // create the service
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // register local app
        let name = ProtoName::from_strings(["cisco", "default", "session"]).with_id(0);
        let (app, _) = service
            .create_app(
                &name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .await
            .expect("failed to create app");

        //////////////////////////// p2p session ////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(Duration::from_millis(500)),
            mls_settings: None,
            initiator: true,
            metadata: HashMap::new(),
        };
        let dst = ProtoName::from_strings(["org", "ns", "dst"]);
        let (session_info, _completion_handle) = app
            .create_session(session_config.clone(), dst, None)
            .await
            .expect("Failed to create session");

        // check the configuration we get is the one we used to create the session
        let session_config_ret = session_info.session().upgrade().unwrap().session_config();

        assert_eq!(session_config_ret, session_config);

        ////////////// multicast session //////////////////////////////////////////////////////////////////////////////////

        let stream = ProtoName::from_strings(["agntcy", "ns", "stream"]);

        let session_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::Multicast,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(1000)),
            mls_settings: Some(MlsSettings::default()),
            initiator: true,
            metadata: HashMap::new(),
        };
        let (session_info, _completion_handle) = app
            .create_session(session_config.clone(), stream.clone(), None)
            .await
            .expect("Failed to create session");

        // The multicast session was created successfully

        let session_config_ret = session_info.session().upgrade().unwrap().session_config();

        assert_eq!(session_config_ret, session_config);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_error_routing_with_session_context() {
        // This test verifies that errors from the datapath that include session context
        // are properly routed to the correct session by sending a message to a non-existent
        // destination and verifying the error contains session context

        use slim_datapath::api::ProtoSessionType;

        info!("starting test_error_routing_with_session_context");

        // Create the service
        let service = Service::new(
            ID::new_with_name(Kind::new(KIND).unwrap(), "test-error-routing").unwrap(),
        );

        // Create an app
        let app_name = ProtoName::from_strings(["cisco", "default", "testapp"]).with_id(0);
        let (app, _app_rx) = service
            .create_app(
                &app_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .await
            .expect("failed to create app");

        // Create a point to point session to a non-existent destination
        // This will trigger an error from the datapath
        let non_existent_dst =
            ProtoName::from_strings(["cisco", "default", "nonexistent"]).with_id(999);
        let mut session_config =
            SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
        session_config.initiator = true;
        session_config.max_retries = Some(10);
        session_config.interval = Some(Duration::from_secs(2));

        let (session, completion_handle) = app
            .create_session(session_config, non_existent_dst.clone(), None)
            .await
            .unwrap();

        let session_id = session.session_arc().unwrap().id();
        info!("Created session with ID: {}", session_id);

        // Wait session creation in completion handle. It should fail quickly as the
        // destination does not exist
        let result = tokio::time::timeout(std::time::Duration::from_millis(300), completion_handle)
            .await
            .expect("timeout waiting for session creation");

        assert!(
            result.is_err_and(|e| {
                println!("--> {}", e.chain());
                true
            }),
            "Session creation should fail for non-existent destination"
        );
    }

    // ── relay_peer_publishes topology tests ─────────────────────────────

    #[test]
    fn test_relay_peer_publishes_full_mesh_is_false() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config = ServerConfig::with_endpoint("0.0.0.0:0").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new()
            .with_dataplane_server(vec![server_config])
            .with_peers(PeerConfig {
                topology: PeerTopology::FullMesh,
                discovery: PeerDiscoveryConfig::Static {
                    peers: vec![StaticPeerEntry {
                        node_id: "peer-1".to_string(),
                        config: ClientConfig::with_endpoint("http://127.0.0.1:9999"),
                    }],
                },
            });
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test-fm").unwrap())
            .unwrap();

        // In FullMesh, peers receive subscriptions directly, so relay is disabled.
        assert!(!service.message_processor().relay_peer_publishes());
    }

    #[test]
    fn test_relay_peer_publishes_no_peers_is_true() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config = ServerConfig::with_endpoint("0.0.0.0:0").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server(vec![server_config]);
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test-no-peers").unwrap())
            .unwrap();

        // Without peer config (standalone), relay is enabled.
        assert!(service.message_processor().relay_peer_publishes());
    }

    // ── peer sync config construction ───────────────────────────────────

    #[tokio::test]
    #[traced_test]
    async fn test_peer_sync_starts_with_static_discovery() {
        let port = slim_testing::common::reserve_local_port();

        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint(&format!("0.0.0.0:{port}")).with_tls_settings(tls_config);
        let peer_config = PeerConfig {
            topology: PeerTopology::FullMesh,
            discovery: PeerDiscoveryConfig::Static {
                peers: vec![StaticPeerEntry {
                    node_id: "other-node".to_string(),
                    config: ClientConfig::with_endpoint("http://127.0.0.1:19999"),
                }],
            },
        };
        let mut svc_config = ServiceConfiguration::new();
        svc_config.node_id = "self-node".to_string();
        let svc_config = svc_config
            .with_dataplane_server(vec![server_config])
            .with_peers(peer_config);

        let service = svc_config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test-static").unwrap())
            .unwrap();

        service.run().await.expect("service should start");
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(logs_contain("starting peer sync"));

        service.shutdown().await.ok();
    }
}
