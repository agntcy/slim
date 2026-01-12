// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, OnceLock};

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
use slim_config::component::id::{ID, Kind};
use slim_config::component::{Component, ComponentBuilder};
use slim_config::grpc::client::ClientConfig as CoreClientConfig;
use slim_config::grpc::server::ServerConfig as CoreServerConfig;
use slim_controller::config::Config as CoreControllerConfig;
use slim_datapath::messages::Name as SlimName;
use slim_service::{
    KIND, Service as SlimService, ServiceConfiguration as SlimServiceConfiguration,
};

use crate::name::Name;

// Global static service instance for bindings
static GLOBAL_SERVICE: OnceLock<Arc<Service>> = OnceLock::new();

/// Get or initialize the global service for bindings
pub fn get_or_init_global_service() -> Arc<Service> {
    GLOBAL_SERVICE
        .get_or_init(|| {
            let slim_service = SlimService::builder()
                .build("global-bindings-service".to_string())
                .expect("Failed to create global bindings service");
            Arc::new(Service {
                inner: Arc::new(RwLock::new(slim_service)),
            })
        })
        .clone()
}

/// DataPlane configuration wrapper for uniffi bindings
#[derive(Clone, uniffi::Record)]
pub struct DataplaneConfig {
    /// DataPlane GRPC server settings
    pub servers: Vec<ServerConfig>,
    /// DataPlane client configs
    pub clients: Vec<ClientConfig>,
}

impl Default for DataplaneConfig {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            clients: Vec::new(),
        }
    }
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
#[derive(Clone, uniffi::Record)]
pub struct ServiceConfiguration {
    pub node_id: Option<String>,
    pub group_name: Option<String>,
    pub dataplane: DataplaneConfig,
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        Self {
            node_id: None,
            group_name: None,
            dataplane: DataplaneConfig::default(),
        }
    }
}

impl From<ServiceConfiguration> for SlimServiceConfiguration {
    fn from(config: ServiceConfiguration) -> Self {
        let mut core_config = SlimServiceConfiguration::new();
        core_config.node_id = config.node_id;
        core_config.group_name = config.group_name;
        core_config.dataplane = config.dataplane.into();
        core_config
    }
}

impl From<SlimServiceConfiguration> for ServiceConfiguration {
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
    pub fn new_with_config(name: String, config: ServiceConfiguration) -> Self {
        let kind = Kind::new(KIND).expect("Invalid service kind");
        let id = ID::new_with_name(kind, &name).expect("Invalid service name");
        let core_config: SlimServiceConfiguration = config.into();
        let service = SlimService::new_with_config(id, core_config);
        Service {
            inner: Arc::new(RwLock::new(service)),
        }
    }

    /// Get the service configuration
    pub async fn config(&self) -> ServiceConfiguration {
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

    /// Create a new BindingsAdapter with authentication configuration (async version)
    ///
    /// This method initializes authentication providers/verifiers and creates a BindingsAdapter
    /// on this service instance.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    ///
    /// # Returns
    /// * `Ok(Arc<BindingsAdapter>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    pub async fn create_adapter_async(
        &self,
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Arc<crate::adapter::BindingsAdapter>, SlimError> {
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

        Ok(Arc::new(crate::adapter::BindingsAdapter::from_parts(
            Arc::new(app),
            Arc::new(RwLock::new(rx)),
            self.inner.clone(),
        )))
    }
}

/// Create a new ServiceConfiguration
#[uniffi::export]
pub fn new_service_configuration() -> ServiceConfiguration {
    ServiceConfiguration::new()
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
    config: ServiceConfiguration,
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

/// Get the global service configuration
#[uniffi::export]
pub async fn service_config() -> ServiceConfiguration {
    get_or_init_global_service().config().await
}

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
    get_or_init_global_service().get_connection_id(endpoint).await
}

/// Create a new BindingsAdapter with authentication on the global service
///
/// This is a convenience function that creates an adapter using the global service instance.
///
/// # Arguments
/// * `base_name` - The base name for the app (without ID)
/// * `identity_provider_config` - Configuration for proving identity to others
/// * `identity_verifier_config` - Configuration for verifying identity of others
///
/// # Returns
/// * `Ok(Arc<BindingsAdapter>)` - Successfully created adapter
/// * `Err(SlimError)` - If adapter creation fails
#[uniffi::export]
pub async fn create_adapter(
    base_name: Arc<Name>,
    identity_provider_config: IdentityProviderConfig,
    identity_verifier_config: IdentityVerifierConfig,
) -> Result<Arc<crate::adapter::BindingsAdapter>, SlimError> {
    get_or_init_global_service()
        .create_adapter_async(base_name, identity_provider_config, identity_verifier_config)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::messages::Name;
    use slim_testing::utils::TEST_VALID_SECRET;

    use crate::adapter::BindingsAdapter;
    use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};

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
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }

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
    async fn test_service_arc_sharing() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get global service instance
        let global_service = get_or_init_global_service();
        let ptr1 = Arc::as_ptr(&global_service.inner) as usize;

        // Test global service - just ensure it creates without error
        let _global_adapter1 =
            BindingsAdapter::new_async(base_name.clone(), provider.clone(), verifier.clone())
                .await
                .unwrap();

        // Test global service again - ensure it uses the same Arc
        let _global_adapter2 = BindingsAdapter::new_async(base_name, provider, verifier)
            .await
            .unwrap();

        let global_service2 = get_or_init_global_service();
        let ptr2 = Arc::as_ptr(&global_service2.inner) as usize;
        
        // They should point to the same inner Arc
        assert_eq!(ptr1, ptr2);
    }
}
