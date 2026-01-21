// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::error::{Result, SRPCError};
use crate::{App, IdentityProviderConfig, IdentityVerifierConfig};
use std::sync::Arc;

/// Configuration for a SLIM RPC application
#[derive(Debug, Clone, uniffi::Record)]
pub struct RpcAppConfig {
    pub identity: String,
    pub endpoint: String,
    pub shared_secret: String,
    pub enable_opentelemetry: bool,
}

/// Result of creating and connecting an app
#[derive(uniffi::Record)]
pub struct RpcAppConnection {
    pub app: Arc<App>,
    pub connection_id: u64,
}

impl RpcAppConfig {
    /// Create a new SLIMAppConfig with SharedSecret authentication
    pub fn with_shared_secret(
        identity: impl Into<String>,
        endpoint: impl Into<String>,
        shared_secret: impl Into<String>,
    ) -> Self {
        Self {
            identity: identity.into(),
            endpoint: endpoint.into(),
            shared_secret: shared_secret.into(),
            enable_opentelemetry: false,
        }
    }
    
    /// Enable or disable OpenTelemetry
    pub fn with_opentelemetry(mut self, enabled: bool) -> Self {
        self.enable_opentelemetry = enabled;
        self
    }
}

/// Create a new RpcAppConfig with SharedSecret authentication (for FFI)
#[uniffi::export]
pub fn new_rpc_app_config(identity: String, endpoint: String, shared_secret: String) -> RpcAppConfig {
    RpcAppConfig::with_shared_secret(identity, endpoint, shared_secret)
}

/// Split an ID into its components
/// Expected format: organization/namespace/application
fn split_id(id: &str) -> Result<slim_datapath::messages::Name> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() < 3 {
        return Err(SRPCError::InvalidId(format!(
            "ID must be in format organization/namespace/app, got: {}",
            id
        )));
    }

    Ok(slim_datapath::messages::Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0))
}

/// Create and connect app (blocking)
#[uniffi::export]
pub fn create_and_connect_app(config: RpcAppConfig) -> Result<RpcAppConnection> {
    crate::get_runtime().block_on(async {
        create_and_connect_app_async(config).await
    })
}

/// Create and connect app (async)
#[uniffi::export]
pub async fn create_and_connect_app_async(config: RpcAppConfig) -> Result<RpcAppConnection> {
    use crate::{get_global_service, initialize_with_defaults, Name as BindingsName};
    
    // Initialize crypto, runtime, global service (like Go's InitializeWithDefaults)
    initialize_with_defaults();
    
    // Parse identity to Name
    let app_name = split_id(&config.identity)?;
    let bindings_name = Arc::new(BindingsName::from(&app_name));
    
    // Create app using async bindings API (avoids nested runtime issue)
    let identity_provider = IdentityProviderConfig::SharedSecret {
        id: config.identity.clone(),
        data: config.shared_secret.clone(),
    };
    let identity_verifier = IdentityVerifierConfig::SharedSecret {
        id: config.identity.clone(),
        data: config.shared_secret.clone(),
    };
    
    let app = App::new_async(
        (*bindings_name).clone().into(),
        identity_provider,
        identity_verifier,
    ).await
    .map(Arc::new)
    .map_err(|e| SRPCError::ParseIdentity(format!("App creation failed: {}", e)))?;
    
    // Create client config
    let client_config = crate::new_insecure_client_config(config.endpoint);
    
    // Connect to SLIM server (async to avoid nested runtime)
    let conn_id = get_global_service()
        .connect_async(client_config)
        .await
        .map_err(|e| SRPCError::ParseIdentity(format!("Connection failed: {}", e)))?;
    
    // Subscribe to local name (async to avoid nested runtime)
    app.subscribe_async(app.name(), Some(conn_id))
        .await
        .map_err(|e| SRPCError::ParseIdentity(format!("Subscription failed: {}", e)))?;
    
    Ok(RpcAppConnection {
        app,
        connection_id: conn_id,
    })
}
