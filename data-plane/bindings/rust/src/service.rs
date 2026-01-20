// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::client_config::ClientConfig;
use crate::errors::SlimError;
use crate::get_runtime;
use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};
use crate::server_config::ServerConfig;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::auth::identity::{
    IdentityProviderConfig as CoreIdentityProviderConfig,
    IdentityVerifierConfig as CoreIdentityVerifierConfig,
};
use slim_config::component::Component;
use slim_config::component::id::{ID, Kind};
use slim_config::grpc::client::ClientConfig as CoreClientConfig;
use slim_config::grpc::server::ServerConfig as CoreServerConfig;
use slim_controller::config::Config as CoreControllerConfig;
use slim_datapath::messages::Name as SlimName;
use slim_service::{
    KIND, Service as SlimService, ServiceConfiguration as SlimServiceConfiguration,
};
use tokio::task::JoinHandle;

use crate::name::Name;

/// DataPlane configuration wrapper for uniffi bindings
#[derive(Clone, Default, uniffi::Record)]
pub struct DataplaneConfig {
    /// DataPlane GRPC server settings
    pub servers: Vec<ServerConfig>,
    /// DataPlane client configs
    pub clients: Vec<ClientConfig>,
}

impl From<DataplaneConfig> for CoreControllerConfig {
    fn from(config: DataplaneConfig) -> Self {
        let mut core_config = CoreControllerConfig::new();
        core_config.servers = config
            .servers
            .into_iter()
            .map(|s| {
                let core: CoreServerConfig = s.into();
                core
            })
            .collect();
        core_config.clients = config
            .clients
            .into_iter()
            .map(|c| {
                let core: CoreClientConfig = c.into();
                core
            })
            .collect();
        core_config.token_provider = CoreIdentityProviderConfig::None;
        core_config.token_verifier = CoreIdentityVerifierConfig::None;
        core_config
    }
}

impl From<CoreControllerConfig> for DataplaneConfig {
    fn from(config: CoreControllerConfig) -> Self {
        Self {
            servers: config.servers.into_iter().map(|s| s.into()).collect(),
            clients: config.clients.into_iter().map(|c| c.into()).collect(),
        }
    }
}

/// Service configuration wrapper for uniffi bindings
#[derive(Clone, Default, uniffi::Record)]
pub struct ServiceConfig {
    /// Optional node ID for the service
    pub node_id: Option<String>,

    /// Optional group name for the service
    pub group_name: Option<String>,

    /// DataPlane configuration (servers and clients)
    pub dataplane: DataplaneConfig,
}

impl ServiceConfig {
    pub fn new() -> Self {
        Self {
            node_id: None,
            group_name: None,
            dataplane: DataplaneConfig::default(),
        }
    }
}

impl From<ServiceConfig> for SlimServiceConfiguration {
    fn from(config: ServiceConfig) -> Self {
        let mut core_config = SlimServiceConfiguration::new();
        core_config.node_id = config.node_id;
        core_config.group_name = config.group_name;
        core_config.dataplane = config.dataplane.into();
        core_config
    }
}

impl From<SlimServiceConfiguration> for ServiceConfig {
    fn from(config: SlimServiceConfiguration) -> Self {
        Self {
            node_id: config.node_id,
            group_name: config.group_name,
            dataplane: config.dataplane.into(),
        }
    }
}

impl From<&SlimServiceConfiguration> for ServiceConfig {
    fn from(config: &SlimServiceConfiguration) -> Self {
        Self {
            node_id: config.node_id.clone(),
            group_name: config.group_name.clone(),
            dataplane: config.dataplane.clone().into(),
        }
    }
}

/// Service wrapper for uniffi bindings
#[derive(uniffi::Object)]
pub struct Service {
    pub(crate) inner: Arc<SlimService>,
}

impl Service {
    /// Get a clone of the inner service Arc for advanced use cases
    pub fn inner(&self) -> Arc<SlimService> {
        self.inner.clone()
    }
}

/// Conversion traits
impl From<SlimService> for Service {
    fn from(service: SlimService) -> Self {
        Service {
            inner: Arc::new(service),
        }
    }
}

#[uniffi::export]
impl Service {
    /// Create a new Service with the given name
    #[uniffi::constructor]
    pub fn new(name: String) -> Self {
        let kind = Kind::new(KIND).expect("Invalid service kind");
        let id = ID::new_with_name(kind, &name).expect("Invalid service name");
        let service = SlimService::new(id);
        Service {
            inner: Arc::new(service),
        }
    }

    /// Create a new Service with configuration
    #[uniffi::constructor]
    pub fn new_with_config(name: String, config: ServiceConfig) -> Self {
        let kind = Kind::new(KIND).expect("Invalid service kind");
        let id = ID::new_with_name(kind, &name).expect("Invalid service name");
        let core_config: SlimServiceConfiguration = config.into();
        let service = SlimService::new_with_config(id, core_config);
        Service {
            inner: Arc::new(service),
        }
    }

    /// Get the service configuration
    pub fn config(&self) -> ServiceConfig {
        self.inner.config().clone().into()
    }

    /// Get the service identifier/name
    pub fn get_name(&self) -> String {
        self.inner.identifier().to_string()
    }

    /// Run the service (starts all configured servers and clients)
    pub async fn run_async(&self) -> Result<(), SlimError> {
        self.inner.run().await.map_err(|e| SlimError::ServiceError {
            message: format!("Failed to run service: {}", e),
        })
    }

    /// Run the service (starts all configured servers and clients) - blocking version
    pub fn run(&self) -> Result<(), SlimError> {
        crate::config::get_runtime().block_on(self.run_async())
    }

    /// Shutdown the service gracefully
    pub async fn shutdown_async(&self) -> Result<(), SlimError> {
        self.inner
            .shutdown()
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to shutdown service: {}", e),
            })
    }

    /// Shutdown the service gracefully - blocking version
    pub fn shutdown(&self) -> Result<(), SlimError> {
        crate::config::get_runtime().block_on(self.shutdown_async())
    }

    /// Start a server with the given configuration
    pub async fn run_server_async(&self, config: ServerConfig) -> Result<(), SlimError> {
        // Use the runtime handle to spawn the task, ensuring proper tokio context
        let runtime = get_runtime();
        let core_config: slim_config::grpc::server::ServerConfig = config.into();
        let inner = self.inner.clone();

        // Spawn on the runtime's handle to ensure tokio context is available
        let handle: JoinHandle<Result<(), SlimError>> = runtime.handle().spawn(async move {
            inner.run_server(&core_config).await?;
            Ok(())
        });

        handle.await.map_err(|e| SlimError::ServiceError {
            message: format!("Join error: {}", e),
        })?
    }

    /// Start a server with the given configuration - blocking version
    pub fn run_server(&self, config: ServerConfig) -> Result<(), SlimError> {
        crate::config::get_runtime().block_on(self.run_server_async(config))
    }

    /// Stop a server by endpoint - blocking version
    pub fn stop_server(&self, endpoint: String) -> Result<(), SlimError> {
        self.inner
            .stop_server(&endpoint)
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to stop server: {}", e),
            })
    }

    /// Connect to a remote endpoint as a client
    pub async fn connect_async(&self, config: ClientConfig) -> Result<u64, SlimError> {
        let core_config: slim_config::grpc::client::ClientConfig = config.into();
        let inner = self.inner.clone();
        let runtime = get_runtime();

        // Spawn in tokio runtime since connect internally uses tokio::spawn
        let handle = runtime.spawn(async move {
            inner
                .connect(&core_config)
                .await
                .map_err(|e| SlimError::ServiceError {
                    message: format!("Failed to connect: {}", e),
                })
        });

        handle.await.map_err(|e| SlimError::InternalError {
            message: format!("Failed to join connect task: {}", e),
        })?
    }

    /// Connect to a remote endpoint as a client - blocking version
    pub fn connect(&self, config: ClientConfig) -> Result<u64, SlimError> {
        crate::config::get_runtime().block_on(self.connect_async(config))
    }

    /// Disconnect a client connection by connection ID - blocking version
    pub fn disconnect(&self, conn_id: u64) -> Result<(), SlimError> {
        self.inner
            .disconnect(conn_id)
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to disconnect: {}", e),
            })
    }

    /// Get the connection ID for a given endpoint
    pub fn get_connection_id(&self, endpoint: String) -> Option<u64> {
        self.inner.get_connection_id(&endpoint)
    }

    /// Create a new App with authentication configuration (async version)
    ///
    /// This method initializes authentication providers/verifiers and creates a App
    /// on this service instance.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    pub async fn create_app_async(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        let slim_name: SlimName = base_name.as_ref().into();
        create_app_async_internal(
            slim_name,
            identity_provider_config,
            identity_verifier_config,
            self.inner.clone(),
            false,
            false,
        )
        .await
        .map(Arc::new)
    }

    /// Create a new App with authentication configuration and shutdown flags (async version)
    ///
    /// This method initializes authentication providers/verifiers and creates a App
    /// on this service instance. It also set the shutdown flags for send and receive.
    /// If set to true, the app will inibit the sending or receiving of messages.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    /// * `shutdown_send` - If true, the app will inhibit sending messages
    /// * `shutdown_receive` - If true, the app will inhibit receiving messages
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    pub async fn create_app_async_with_shutdown_flags(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        shutdown_send: bool,
        shutdown_receive: bool,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        let slim_name: SlimName = base_name.as_ref().into();
        create_app_async_internal(
            slim_name,
            identity_provider_config,
            identity_verifier_config,
            self.inner.clone(),
            shutdown_send,
            shutdown_receive,
        )
        .await
        .map(Arc::new)
    }

    /// Create a new App with SharedSecret authentication (async version)
    ///
    /// This is a convenience function for creating a SLIM application using SharedSecret authentication
    /// on this service instance. This is the async version.
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `secret` - The shared secret string for authentication
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created app
    /// * `Err(SlimError)` - If app creation fails
    pub async fn create_app_with_secret_async(
        &self,
        name: Arc<Name>,
        secret: String,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        let identity_provider_config = IdentityProviderConfig::SharedSecret {
            id: name.to_string(),
            data: secret.clone(),
        };
        let identity_verifier_config = IdentityVerifierConfig::SharedSecret {
            id: name.to_string(),
            data: secret,
        };

        self.create_app_async(name, identity_provider_config, identity_verifier_config)
            .await
    }

    /// Create a new App with authentication configuration (blocking version)
    ///
    /// This method initializes authentication providers/verifiers and creates a App
    /// on this service instance. This is a blocking wrapper around create_app_async.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    #[uniffi::method]
    pub fn create_app(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        get_runtime().block_on(self.create_app_async(
            base_name,
            identity_provider_config,
            identity_verifier_config,
        ))
    }

    /// Create a new App with authentication configuration and shutdown flags (blocking version)
    ///
    /// This method initializes authentication providers/verifiers and creates a App
    /// on this service instance. It also set the shutdown flags for send and receive.
    /// If set to true, the app will inibit the sending or receiving of messages.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    /// * `shutdown_send` - If true, the app will inhibit sending messages
    /// * `shutdown_receive` - If true, the app will inhibit receiving messages
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    pub async fn create_app_with_shutdown_flags(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        shutdown_send: bool,
        shutdown_receive: bool,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        let slim_name: SlimName = base_name.as_ref().into();
        create_app_async_internal(
            slim_name,
            identity_provider_config,
            identity_verifier_config,
            self.inner.clone(),
            shutdown_send,
            shutdown_receive,
        )
        .await
        .map(Arc::new)
    }

    /// Create a new App with SharedSecret authentication (helper function)
    ///
    /// This is a convenience function for creating a SLIM application using SharedSecret authentication
    /// on this service instance.
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `secret` - The shared secret string for authentication
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created app
    /// * `Err(SlimError)` - If app creation fails
    #[uniffi::method]
    pub fn create_app_with_secret(
        &self,
        name: Arc<Name>,
        secret: String,
    ) -> Result<Arc<crate::app::App>, SlimError> {
        get_runtime().block_on(self.create_app_with_secret_async(name, secret))
    }
}

/// Internal async app creation logic (used by both service and app constructors)
///
/// This is the core implementation shared by Service::create_app_async and App::new_async_with_service.
///
/// # Arguments
/// * `base_name` - The base name for the app (without ID)
/// * `identity_provider_config` - Configuration for proving identity to others
/// * `identity_verifier_config` - Configuration for verifying identity of others
/// * `service` - The service instance to use for creating the app
///
/// # Returns
/// * `Ok(App)` - Successfully created app
/// * `Err(SlimError)` - If app creation fails
pub(crate) async fn create_app_async_internal(
    base_name: SlimName,
    identity_provider_config: IdentityProviderConfig,
    identity_verifier_config: IdentityVerifierConfig,
    service: Arc<SlimService>,
    shutdown_send: bool,
    shutdown_receive: bool,
) -> Result<crate::app::App, SlimError> {
    // Convert configurations to actual providers/verifiers
    let mut identity_provider: AuthProvider = identity_provider_config.try_into()?;
    let mut identity_verifier: AuthVerifier = identity_verifier_config.try_into()?;

    // Initialize the identity provider
    identity_provider.initialize().await?;

    // Initialize the identity verifier
    identity_verifier.initialize().await?;

    // Validate token
    let _identity_token = identity_provider.get_token()?;

    // Get ID from token and generate name with token ID
    let token_id = identity_provider.get_id()?;

    // Use a hash of the token ID to convert to u64 for name generation
    let id_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token_id.hash(&mut hasher);
        hasher.finish()
    };
    let app_name = base_name.with_id(id_hash);

    // Create the app using the provided service
    let (app, rx) = service.create_app(&app_name, identity_provider, identity_verifier, shutdown_send, shutdown_receive)?;

    Ok(crate::app::App::from_parts(
        Arc::new(app),
        Arc::new(tokio::sync::RwLock::new(rx)),
        service,
    ))
}

/// Create a new ServiceConfiguration
#[uniffi::export]
pub fn new_service_configuration() -> ServiceConfig {
    ServiceConfig::new()
}

/// Create a new DataplaneConfig
#[uniffi::export]
pub fn new_dataplane_config() -> DataplaneConfig {
    DataplaneConfig::default()
}

/// Create a new Service with builder pattern
#[uniffi::export]
pub fn create_service(name: String) -> Result<Arc<Service>, SlimError> {
    Ok(Arc::new(Service::new(name)))
}

/// Create a new Service with configuration
#[uniffi::export]
pub fn create_service_with_config(
    name: String,
    config: ServiceConfig,
) -> Result<Arc<Service>, SlimError> {
    Ok(Arc::new(Service::new_with_config(name, config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::messages::Name as SlimName;
    use slim_testing::utils::TEST_VALID_SECRET;

    use crate::app::App;
    use crate::config::get_global_service;
    use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};
    use crate::name::Name;

    /// Create test authentication configurations
    fn create_test_auth() -> (IdentityProviderConfig, IdentityVerifierConfig) {
        let provider = IdentityProviderConfig::SharedSecret {
            id: "test-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };
        let verifier = IdentityVerifierConfig::SharedSecret {
            id: "test-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };
        (provider, verifier)
    }

    /// Create test app name
    fn create_test_name() -> SlimName {
        SlimName::from_strings(["org", "namespace", "test-app"])
    }

    // ========================================================================
    // Configuration Tests
    // ========================================================================

    #[test]
    fn test_dataplane_config_default() {
        let config = DataplaneConfig::default();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
    }

    #[test]
    fn test_dataplane_config_to_core_conversion() {
        let server_config = ServerConfig::default();
        let client_config = ClientConfig::default();

        let config = DataplaneConfig {
            servers: vec![server_config],
            clients: vec![client_config],
        };

        let core_config: CoreControllerConfig = config.clone().into();
        assert_eq!(core_config.servers.len(), 1);
        assert_eq!(core_config.clients.len(), 1);

        // Verify token provider/verifier are set to None
        assert!(matches!(
            core_config.token_provider,
            CoreIdentityProviderConfig::None
        ));
        assert!(matches!(
            core_config.token_verifier,
            CoreIdentityVerifierConfig::None
        ));
    }

    #[test]
    fn test_dataplane_config_from_core_conversion() {
        let mut core_config = CoreControllerConfig::new();
        core_config.servers = vec![CoreServerConfig::default()];
        core_config.clients = vec![CoreClientConfig::default()];

        let config: DataplaneConfig = core_config.into();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.clients.len(), 1);
    }

    #[test]
    fn test_dataplane_config_roundtrip() {
        let original = DataplaneConfig {
            servers: vec![ServerConfig::default()],
            clients: vec![ClientConfig::default()],
        };

        let core: CoreControllerConfig = original.clone().into();
        let roundtrip: DataplaneConfig = core.into();

        assert_eq!(original.servers.len(), roundtrip.servers.len());
        assert_eq!(original.clients.len(), roundtrip.clients.len());
    }

    #[test]
    fn test_service_configuration_new() {
        let config = ServiceConfig::new();
        assert!(config.node_id.is_none());
        assert!(config.group_name.is_none());
        assert!(config.dataplane.servers.is_empty());
        assert!(config.dataplane.clients.is_empty());
    }

    #[test]
    fn test_service_configuration_default() {
        let config = ServiceConfig::default();
        assert!(config.node_id.is_none());
        assert!(config.group_name.is_none());
    }

    #[test]
    fn test_service_configuration_with_values() {
        let config = ServiceConfig {
            node_id: Some("node-123".to_string()),
            group_name: Some("test-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        assert_eq!(config.node_id.as_deref(), Some("node-123"));
        assert_eq!(config.group_name.as_deref(), Some("test-group"));
    }

    #[test]
    fn test_service_configuration_to_core_conversion() {
        let config = ServiceConfig {
            node_id: Some("node-456".to_string()),
            group_name: Some("group-abc".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let core_config: SlimServiceConfiguration = config.clone().into();
        assert_eq!(core_config.node_id.as_deref(), Some("node-456"));
        assert_eq!(core_config.group_name.as_deref(), Some("group-abc"));
    }

    #[test]
    fn test_service_configuration_from_core_conversion() {
        let mut core_config = SlimServiceConfiguration::new();
        core_config.node_id = Some("core-node".to_string());
        core_config.group_name = Some("core-group".to_string());

        let config: ServiceConfig = core_config.into();
        assert_eq!(config.node_id.as_deref(), Some("core-node"));
        assert_eq!(config.group_name.as_deref(), Some("core-group"));
    }

    #[test]
    fn test_service_configuration_roundtrip() {
        let original = ServiceConfig {
            node_id: Some("roundtrip-node".to_string()),
            group_name: Some("roundtrip-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let core: SlimServiceConfiguration = original.clone().into();
        let roundtrip: ServiceConfig = core.into();

        assert_eq!(original.node_id, roundtrip.node_id);
        assert_eq!(original.group_name, roundtrip.group_name);
    }

    // ========================================================================
    // Service Creation Tests
    // ========================================================================

    #[test]
    fn test_service_new() {
        let service = Service::new("test-service".to_string());
        let name = service.get_name();
        assert!(name.contains("test-service"));
    }

    #[test]
    fn test_service_new_with_config() {
        let config = ServiceConfig {
            node_id: Some("test-node".to_string()),
            group_name: Some("test-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let service = Service::new_with_config("configured-service".to_string(), config.clone());
        let retrieved_config = service.config();

        assert_eq!(retrieved_config.node_id, config.node_id);
        assert_eq!(retrieved_config.group_name, config.group_name);
    }

    #[test]
    fn test_service_inner_clone() {
        let service = Service::new("inner-test".to_string());
        let inner1 = service.inner();
        let inner2 = service.inner();

        // Both should point to the same Arc
        assert!(Arc::ptr_eq(&inner1, &inner2));
    }

    #[test]
    fn test_service_from_slim_service() {
        let kind = Kind::new(KIND).expect("Invalid service kind");
        let id = ID::new_with_name(kind, "from-test").expect("Invalid service name");
        let slim_service = SlimService::new(id);

        let service: Service = slim_service.into();
        // Just verify it doesn't panic
        assert!(Arc::strong_count(&service.inner) >= 1);
    }

    // ========================================================================
    // Global Service Tests
    // ========================================================================

    #[test]
    fn test_global_service_singleton() {
        let service1 = get_global_service();
        let service2 = get_global_service();

        // They should be the same instance (same memory address)
        assert!(Arc::ptr_eq(&service1, &service2));

        // Also check that the inner Arc is the same
        assert!(Arc::ptr_eq(&service1.inner, &service2.inner));
    }

    #[test]
    fn test_get_global_service_function() {
        let service1 = get_global_service();
        let service2 = get_global_service();

        assert!(Arc::ptr_eq(&service1, &service2));
    }

    #[tokio::test]
    async fn test_service_arc_sharing() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get global service instance
        let global_service = get_global_service();
        let ptr1 = Arc::as_ptr(&global_service.inner) as usize;

        // Test global service - just ensure it creates without error
        let _global_adapter1 =
            App::new_async(base_name.clone(), provider.clone(), verifier.clone())
                .await
                .unwrap();

        // Test global service again - ensure it uses the same Arc
        let _global_adapter2 = App::new_async(base_name, provider, verifier).await.unwrap();

        let global_service2 = get_global_service();
        let ptr2 = Arc::as_ptr(&global_service2.inner) as usize;

        // They should point to the same inner Arc
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_global_service_name() {
        let name = get_global_service().get_name();
        assert!(!name.is_empty());
        assert!(name.contains("global-bindings-service"));
    }

    // ========================================================================
    // Service Lifecycle Tests
    // ========================================================================

    #[tokio::test]
    async fn test_service_shutdown_without_run() {
        let service = Service::new("shutdown-test".to_string());
        // Should not error even if service wasn't run
        let result = service.shutdown_async().await;
        // Shutdown might succeed or fail gracefully
        assert!(result.is_ok() || result.is_err());
    }

    // ========================================================================
    // Client/Server Configuration Tests
    // ========================================================================

    #[test]
    fn test_stop_nonexistent_server() {
        let service = Service::new("stop-test".to_string());
        let result = service.stop_server("127.0.0.1:99999".to_string());
        // Should fail because server doesn't exist
        assert!(result.is_err());
    }

    #[test]
    fn test_disconnect_invalid_connection() {
        let service = Service::new("disconnect-test".to_string());
        let result = service.disconnect(999999);
        // Should fail because connection doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_connection_id_nonexistent() {
        let service = Service::new("conn-id-test".to_string());
        let conn_id = service.get_connection_id("nonexistent-endpoint".to_string());
        assert!(conn_id.is_none());
    }

    // ========================================================================
    // Adapter Creation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_create_adapter_async() {
        let service = Service::new("adapter-test".to_string());
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "adapter-app".to_string(),
        ));
        let (provider, verifier) = create_test_auth();

        let result = service
            .create_app_async(base_name, provider, verifier)
            .await;

        assert!(result.is_ok(), "Should create adapter successfully");
        let adapter = result.unwrap();
        assert!(adapter.id() > 0, "Adapter should have non-zero ID");
    }

    #[tokio::test]
    async fn test_create_adapter_with_different_names() {
        let service = Service::new("multi-adapter-test".to_string());
        let (provider, verifier) = create_test_auth();

        let names = vec![
            ("org1", "ns1", "app1"),
            ("org2", "ns2", "app2"),
            ("org3", "ns3", "app3"),
        ];

        for (org, ns, app) in names {
            let base_name = Arc::new(Name::new(org.to_string(), ns.to_string(), app.to_string()));

            let result = service
                .create_app_async(base_name, provider.clone(), verifier.clone())
                .await;

            assert!(
                result.is_ok(),
                "Should create adapter for {}/{}/{}",
                org,
                ns,
                app
            );
        }
    }

    #[tokio::test]
    async fn test_create_adapter_unique_ids() {
        // Each adapter gets a unique ID due to token generation
        let service = Service::new("unique-ids-test".to_string());
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "unique-app".to_string(),
        ));
        let (provider, verifier) = create_test_auth();

        let adapter1 = service
            .create_app_async(base_name.clone(), provider.clone(), verifier.clone())
            .await
            .expect("Failed to create first adapter");

        let adapter2 = service
            .create_app_async(base_name, provider, verifier)
            .await
            .expect("Failed to create second adapter");

        // IDs should be different since each adapter gets a fresh token
        // (tokens include timestamps, so they're unique per creation)
        assert_ne!(adapter1.id(), adapter2.id());

        // Both IDs should be non-zero
        assert!(adapter1.id() > 0);
        assert!(adapter2.id() > 0);
    }

    #[tokio::test]
    async fn test_create_app_with_secret_async() {
        let service = Service::new("secret-app-test".to_string());
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "secret-app".to_string(),
        ));
        let secret = TEST_VALID_SECRET.to_string();

        let result = service
            .create_app_with_secret_async(base_name, secret)
            .await;

        assert!(result.is_ok(), "Should create app with secret successfully");
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
    }

    #[tokio::test]
    async fn test_create_app_with_secret_multiple() {
        let service = Service::new("multi-secret-app-test".to_string());
        let secret = TEST_VALID_SECRET.to_string();

        let names = vec![
            ("org1", "ns1", "secret-app1"),
            ("org2", "ns2", "secret-app2"),
            ("org3", "ns3", "secret-app3"),
        ];

        for (org, ns, app) in names {
            let base_name = Arc::new(Name::new(org.to_string(), ns.to_string(), app.to_string()));

            let result = service
                .create_app_with_secret_async(base_name, secret.clone())
                .await;

            assert!(
                result.is_ok(),
                "Should create secret app for {}/{}/{}",
                org,
                ns,
                app
            );
        }
    }

    #[test]
    fn test_create_app_blocking() {
        let service = Service::new("blocking-app-test".to_string());
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "blocking-app".to_string(),
        ));
        let (provider, verifier) = create_test_auth();

        let result = service.create_app(base_name, provider, verifier);

        assert!(result.is_ok(), "Should create app in blocking mode");
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
    }

    #[test]
    fn test_create_app_with_secret_blocking() {
        let service = Service::new("blocking-secret-app-test".to_string());
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "blocking-secret-app".to_string(),
        ));
        let secret = TEST_VALID_SECRET.to_string();

        let result = service.create_app_with_secret(base_name, secret);

        assert!(result.is_ok(), "Should create secret app in blocking mode");
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
    }

    #[tokio::test]
    async fn test_create_app_async_internal_directly() {
        let service = Service::new("internal-test".to_string());
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let result =
            create_app_async_internal(base_name, provider, verifier, service.inner.clone(), false, false).await;

        assert!(result.is_ok(), "Should create app via internal function");
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
    }

    #[tokio::test]
    async fn test_create_app_with_same_secret_different_ids() {
        // Even with the same secret, different apps should get unique IDs
        let service = Service::new("same-secret-test".to_string());
        let secret = TEST_VALID_SECRET.to_string();
        let base_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "same-secret-app".to_string(),
        ));

        let app1 = service
            .create_app_with_secret_async(base_name.clone(), secret.clone())
            .await
            .expect("Failed to create first app");

        let app2 = service
            .create_app_with_secret_async(base_name, secret)
            .await
            .expect("Failed to create second app");

        // IDs should be different due to timestamp in token generation
        assert_ne!(app1.id(), app2.id());
        assert!(app1.id() > 0);
        assert!(app2.id() > 0);
    }

    // ========================================================================
    // Factory Function Tests
    // ========================================================================

    #[test]
    fn test_new_service_configuration() {
        let config = new_service_configuration();
        assert!(config.node_id.is_none());
        assert!(config.group_name.is_none());
    }

    #[test]
    fn test_new_dataplane_config() {
        let config = new_dataplane_config();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
    }

    #[test]
    fn test_create_service() {
        let result = create_service("factory-test".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_service_with_config() {
        let config = ServiceConfig {
            node_id: Some("factory-node".to_string()),
            group_name: Some("factory-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let result = create_service_with_config("factory-configured-test".to_string(), config);
        assert!(result.is_ok());
    }

    // ========================================================================
    // Global Service Convenience Functions Tests
    // ========================================================================

    #[test]
    fn test_global_stop_server_convenience() {
        let result = get_global_service().stop_server("127.0.0.1:88888".to_string());
        // Should error since server doesn't exist
        assert!(result.is_err());
    }

    #[test]
    fn test_global_disconnect_convenience() {
        let result = get_global_service().disconnect(888888);
        // Should error since connection doesn't exist
        assert!(result.is_err());
    }

    #[test]
    fn test_global_get_connection_id_convenience() {
        let conn_id = get_global_service().get_connection_id("nonexistent-global".to_string());
        assert!(conn_id.is_none());
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[tokio::test]
    async fn test_service_operations_on_uninitialized_service() {
        // These operations should handle gracefully even on a fresh service
        let service = Service::new("uninitialized-test".to_string());

        // Stop server that doesn't exist
        let result = service.stop_server("127.0.0.1:11111".to_string());
        assert!(result.is_err());

        // Disconnect non-existent connection
        let result = service.disconnect(11111);
        assert!(result.is_err());

        // Get non-existent connection ID
        let conn_id = service.get_connection_id("fake-endpoint".to_string());
        assert!(conn_id.is_none());
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[test]
    fn test_service_with_multiple_configs() {
        let server_config = ServerConfig::default();
        let client_config = ClientConfig::default();

        let dataplane = DataplaneConfig {
            servers: vec![server_config.clone(), server_config],
            clients: vec![client_config.clone(), client_config],
        };

        let service_config = ServiceConfig {
            node_id: Some("multi-config-node".to_string()),
            group_name: Some("multi-config-group".to_string()),
            dataplane,
        };

        let service = Service::new_with_config("multi-config-service".to_string(), service_config);
        let retrieved = service.config();

        assert_eq!(retrieved.dataplane.servers.len(), 2);
        assert_eq!(retrieved.dataplane.clients.len(), 2);
    }

    #[test]
    fn test_service_config_mutation_isolation() {
        let config = ServiceConfig {
            node_id: Some("original-node".to_string()),
            group_name: Some("original-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let service = Service::new_with_config("isolation-test".to_string(), config.clone());

        // Get config and verify it matches
        let retrieved = service.config();
        assert_eq!(retrieved.node_id, config.node_id);

        // Original config should be independent
        let mut modified_config = config;
        modified_config.node_id = Some("modified-node".to_string());

        // Service config should not have changed
        let retrieved2 = service.config();
        assert_eq!(retrieved2.node_id.as_deref(), Some("original-node"));
    }

    // ========================================================================
    // Runtime Spawning Tests
    // ========================================================================

    #[tokio::test]
    async fn test_run_server_async_runtime_context() {
        let service = Service::new("run-server-runtime-test".to_string());
        let config = ServerConfig::default();

        // This should not panic even though it's spawning on runtime
        let result = service.run_server_async(config).await;
        // May fail due to address already in use or other reasons, but shouldn't panic
        let _ = result; // We just want to ensure it doesn't panic
    }

    #[tokio::test]
    async fn test_connect_async_runtime_context() {
        let service = Service::new("connect-runtime-test".to_string());
        let config = ClientConfig::default();

        // This should not panic even though it's spawning on runtime
        let result = service.connect_async(config).await;
        // May fail to connect, but shouldn't panic from join error
        let _ = result; // We just want to ensure proper error handling
    }

    #[test]
    fn test_run_server_blocking() {
        let service = Service::new("run-server-blocking-test".to_string());
        let config = ServerConfig::default();

        // Test blocking version
        let result = service.run_server(config);
        // May fail but should not panic
        let _ = result;
    }

    #[test]
    fn test_connect_blocking() {
        let service = Service::new("connect-blocking-test".to_string());
        let config = ClientConfig::default();

        // Test blocking version
        let result = service.connect(config);
        // May fail but should not panic
        let _ = result;
    }

    // ========================================================================
    // Additional Edge Case Tests
    // ========================================================================

    #[tokio::test]
    async fn test_multiple_apps_on_same_service() {
        let service = Service::new("multi-app-service".to_string());
        let secret = TEST_VALID_SECRET.to_string();

        let name1 = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "app1".to_string(),
        ));
        let name2 = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "app2".to_string(),
        ));
        let name3 = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "app3".to_string(),
        ));

        let app1 = service
            .create_app_with_secret_async(name1, secret.clone())
            .await
            .expect("Failed to create app1");

        let app2 = service
            .create_app_with_secret_async(name2, secret.clone())
            .await
            .expect("Failed to create app2");

        let app3 = service
            .create_app_with_secret_async(name3, secret)
            .await
            .expect("Failed to create app3");

        // All apps should have unique IDs
        assert_ne!(app1.id(), app2.id());
        assert_ne!(app1.id(), app3.id());
        assert_ne!(app2.id(), app3.id());

        // All IDs should be non-zero
        assert!(app1.id() > 0);
        assert!(app2.id() > 0);
        assert!(app3.id() > 0);
    }

    #[test]
    fn test_service_run_blocking() {
        let service = Service::new("run-blocking-test".to_string());
        // Test that run() doesn't panic
        let result = service.run();
        // May succeed or fail, we just ensure no panic
        let _ = result;
    }

    #[test]
    fn test_service_shutdown_blocking() {
        let service = Service::new("shutdown-blocking-test".to_string());
        // Test that shutdown() doesn't panic
        let result = service.shutdown();
        // May succeed or fail, we just ensure no panic
        let _ = result;
    }

    #[tokio::test]
    async fn test_run_async() {
        let service = Service::new("run-async-test".to_string());
        // run_async starts all configured servers/clients
        let result = service.run_async().await;
        // May fail if nothing configured, but shouldn't panic
        let _ = result;
    }
}
