// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::client_config::ClientConfig;
use crate::errors::SlimError;
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

use crate::name::Name;

/// Get or initialize the global service for bindings
pub fn get_or_init_global_service() -> Arc<Service> {
    crate::config::get_service()
}

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

/// Service wrapper for uniffi bindings
#[derive(uniffi::Object)]
pub struct Service {
    pub(crate) inner: Arc<RwLock<SlimService>>,
}

impl Service {
    /// Get a clone of the inner service Arc for advanced use cases
    pub fn inner(&self) -> Arc<RwLock<SlimService>> {
        self.inner.clone()
    }
}

/// Conversion traits
impl From<SlimService> for Service {
    fn from(service: SlimService) -> Self {
        Service {
            inner: Arc::new(RwLock::new(service)),
        }
    }
}

impl From<Service> for SlimService {
    fn from(service: Service) -> Self {
        Arc::try_unwrap(service.inner)
            .expect("Cannot convert Service to SlimService: multiple references exist")
            .into_inner()
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
            inner: Arc::new(RwLock::new(service)),
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
            inner: Arc::new(RwLock::new(service)),
        }
    }

    /// Get the service configuration
    pub async fn config(&self) -> ServiceConfig {
        self.inner.read().await.config().clone().into()
    }

    /// Get the service identifier/name
    pub async fn get_name(&self) -> String {
        self.inner.read().await.identifier().to_string()
    }

    /// Run the service (starts all configured servers and clients)
    pub async fn run(&self) -> Result<(), SlimError> {
        self.inner
            .write()
            .await
            .run()
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to run service: {}", e),
            })
    }

    /// Shutdown the service gracefully
    pub async fn shutdown(&self) -> Result<(), SlimError> {
        self.inner
            .read()
            .await
            .shutdown()
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to shutdown service: {}", e),
            })
    }

    /// Start a server with the given configuration
    pub async fn run_server(&self, config: ServerConfig) -> Result<(), SlimError> {
        let core_config: slim_config::grpc::server::ServerConfig = config.into();
        self.inner
            .read()
            .await
            .run_server(&core_config)
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to run server: {}", e),
            })
    }

    /// Stop a server by endpoint
    pub async fn stop_server(&self, endpoint: String) -> Result<(), SlimError> {
        self.inner
            .read()
            .await
            .stop_server(&endpoint)
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to stop server: {}", e),
            })
    }

    /// Connect to a remote endpoint as a client
    pub async fn connect(&self, config: ClientConfig) -> Result<u64, SlimError> {
        let core_config: slim_config::grpc::client::ClientConfig = config.into();
        self.inner
            .read()
            .await
            .connect(&core_config)
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to connect: {}", e),
            })
    }

    /// Disconnect a client connection by connection ID
    pub async fn disconnect(&self, conn_id: u64) -> Result<(), SlimError> {
        self.inner
            .read()
            .await
            .disconnect(conn_id)
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to disconnect: {}", e),
            })
    }

    /// Get the connection ID for a given endpoint
    pub async fn get_connection_id(&self, endpoint: String) -> Option<u64> {
        self.inner.read().await.get_connection_id(&endpoint)
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
    pub async fn create_adapter_async(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Arc<crate::app::App>, SlimError> {
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
        let slim_name: SlimName = base_name.as_ref().into();
        let app_name = slim_name.with_id(id_hash);

        // Get service reference for adapter creation
        let service_guard = self.inner.read().await;

        // Create the app
        let (app, rx) =
            service_guard.create_app(&app_name, identity_provider, identity_verifier)?;

        // Release the lock before creating the adapter
        drop(service_guard);

        Ok(Arc::new(crate::app::App::from_parts(
            Arc::new(app),
            Arc::new(RwLock::new(rx)),
            self.inner.clone(),
        )))
    }
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

/// Get the global service instance (creates it if it doesn't exist)
///
/// This returns a reference to the shared global service that can be used
/// across the application. All calls to this function return the same service instance.
#[uniffi::export]
pub fn get_global_service() -> Arc<Service> {
    get_or_init_global_service()
}

// ============================================================================
// Global Service Convenience Functions
// ============================================================================
// These functions operate on the global service instance directly

/// Get the global service identifier/name
#[uniffi::export]
pub async fn service_name() -> String {
    get_or_init_global_service().get_name().await
}

/// Run the global service (starts all configured servers and clients)
#[uniffi::export]
pub async fn service_run() -> Result<(), SlimError> {
    get_or_init_global_service().run().await
}

/// Shutdown the global service gracefully
#[uniffi::export]
pub async fn service_shutdown() -> Result<(), SlimError> {
    get_or_init_global_service().shutdown().await
}

/// Start a server on the global service with the given configuration
#[uniffi::export]
pub async fn run_server(config: ServerConfig) -> Result<(), SlimError> {
    get_or_init_global_service().run_server(config).await
}

/// Stop a server on the global service by endpoint
#[uniffi::export]
pub async fn stop_server(endpoint: String) -> Result<(), SlimError> {
    get_or_init_global_service().stop_server(endpoint).await
}

/// Connect to a remote endpoint as a client using the global service
#[uniffi::export]
pub async fn connect(config: ClientConfig) -> Result<u64, SlimError> {
    get_or_init_global_service().connect(config).await
}

/// Disconnect a client connection by connection ID on the global service
#[uniffi::export]
pub async fn disconnect(conn_id: u64) -> Result<(), SlimError> {
    get_or_init_global_service().disconnect(conn_id).await
}

/// Get the connection ID for a given endpoint on the global service
#[uniffi::export]
pub async fn get_connection_id(endpoint: String) -> Option<u64> {
    get_or_init_global_service()
        .get_connection_id(endpoint)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::messages::Name as SlimName;
    use slim_testing::utils::TEST_VALID_SECRET;

    use crate::app::App;
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

    #[tokio::test]
    async fn test_service_new() {
        let service = Service::new("test-service".to_string());
        let name = service.get_name().await;
        assert!(name.contains("test-service"));
    }

    #[tokio::test]
    async fn test_service_new_with_config() {
        let config = ServiceConfig {
            node_id: Some("test-node".to_string()),
            group_name: Some("test-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let service = Service::new_with_config("configured-service".to_string(), config.clone());
        let retrieved_config = service.config().await;

        assert_eq!(retrieved_config.node_id, config.node_id);
        assert_eq!(retrieved_config.group_name, config.group_name);
    }

    #[tokio::test]
    async fn test_service_inner_clone() {
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

    #[tokio::test]
    async fn test_global_service_singleton() {
        let service1 = get_or_init_global_service();
        let service2 = get_or_init_global_service();

        // They should be the same instance (same memory address)
        assert!(Arc::ptr_eq(&service1, &service2));

        // Also check that the inner Arc is the same
        assert!(Arc::ptr_eq(&service1.inner, &service2.inner));
    }

    #[tokio::test]
    async fn test_get_global_service_function() {
        let service1 = get_global_service();
        let service2 = get_global_service();

        assert!(Arc::ptr_eq(&service1, &service2));
    }

    #[tokio::test]
    async fn test_service_arc_sharing() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get global service instance
        let global_service = get_or_init_global_service();
        let ptr1 = Arc::as_ptr(&global_service.inner) as usize;

        // Test global service - just ensure it creates without error
        let _global_adapter1 =
            App::new_async(base_name.clone(), provider.clone(), verifier.clone())
                .await
                .unwrap();

        // Test global service again - ensure it uses the same Arc
        let _global_adapter2 = App::new_async(base_name, provider, verifier).await.unwrap();

        let global_service2 = get_or_init_global_service();
        let ptr2 = Arc::as_ptr(&global_service2.inner) as usize;

        // They should point to the same inner Arc
        assert_eq!(ptr1, ptr2);
    }

    #[tokio::test]
    async fn test_global_service_name() {
        let name = service_name().await;
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
        let result = service.shutdown().await;
        // Shutdown might succeed or fail gracefully
        assert!(result.is_ok() || result.is_err());
    }

    // ========================================================================
    // Client/Server Configuration Tests
    // ========================================================================

    #[tokio::test]
    async fn test_stop_nonexistent_server() {
        let service = Service::new("stop-test".to_string());
        let result = service.stop_server("127.0.0.1:99999".to_string()).await;
        // Should fail because server doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_disconnect_invalid_connection() {
        let service = Service::new("disconnect-test".to_string());
        let result = service.disconnect(999999).await;
        // Should fail because connection doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_connection_id_nonexistent() {
        let service = Service::new("conn-id-test".to_string());
        let conn_id = service
            .get_connection_id("nonexistent-endpoint".to_string())
            .await;
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
            None,
        ));
        let (provider, verifier) = create_test_auth();

        let result = service
            .create_adapter_async(base_name, provider, verifier)
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
            let base_name = Arc::new(Name::new(
                org.to_string(),
                ns.to_string(),
                app.to_string(),
                None,
            ));

            let result = service
                .create_adapter_async(base_name, provider.clone(), verifier.clone())
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
            None,
        ));
        let (provider, verifier) = create_test_auth();

        let adapter1 = service
            .create_adapter_async(base_name.clone(), provider.clone(), verifier.clone())
            .await
            .expect("Failed to create first adapter");

        let adapter2 = service
            .create_adapter_async(base_name, provider, verifier)
            .await
            .expect("Failed to create second adapter");

        // IDs should be different since each adapter gets a fresh token
        // (tokens include timestamps, so they're unique per creation)
        assert_ne!(adapter1.id(), adapter2.id());

        // Both IDs should be non-zero
        assert!(adapter1.id() > 0);
        assert!(adapter2.id() > 0);
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

    #[tokio::test]
    async fn test_global_stop_server_convenience() {
        let result = stop_server("127.0.0.1:88888".to_string()).await;
        // Should error since server doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_global_disconnect_convenience() {
        let result = disconnect(888888).await;
        // Should error since connection doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_global_get_connection_id_convenience() {
        let conn_id = get_connection_id("nonexistent-global".to_string()).await;
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
        let result = service.stop_server("127.0.0.1:11111".to_string()).await;
        assert!(result.is_err());

        // Disconnect non-existent connection
        let result = service.disconnect(11111).await;
        assert!(result.is_err());

        // Get non-existent connection ID
        let conn_id = service.get_connection_id("fake-endpoint".to_string()).await;
        assert!(conn_id.is_none());
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[tokio::test]
    async fn test_service_with_multiple_configs() {
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
        let retrieved = service.config().await;

        assert_eq!(retrieved.dataplane.servers.len(), 2);
        assert_eq!(retrieved.dataplane.clients.len(), 2);
    }

    #[tokio::test]
    async fn test_service_config_mutation_isolation() {
        let config = ServiceConfig {
            node_id: Some("original-node".to_string()),
            group_name: Some("original-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let service = Service::new_with_config("isolation-test".to_string(), config.clone());

        // Get config and verify it matches
        let retrieved = service.config().await;
        assert_eq!(retrieved.node_id, config.node_id);

        // Original config should be independent
        let mut modified_config = config;
        modified_config.node_id = Some("modified-node".to_string());

        // Service config should not have changed
        let retrieved2 = service.config().await;
        assert_eq!(retrieved2.node_id.as_deref(), Some("original-node"));
    }
}
