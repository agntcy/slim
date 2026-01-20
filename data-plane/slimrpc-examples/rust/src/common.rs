// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common utilities for slimrpc examples

use slim_bindings::slimrpc::error::{Result, SRPCError};
use slim_bindings::{ClientConfig, IdentityProviderConfig, IdentityVerifierConfig};
use slim_datapath::messages::Name;

/// Configuration for a SLIM RPC application
#[derive(Debug, Clone)]
pub struct SLIMAppConfig {
    pub identity: String,
    pub identity_provider: IdentityProviderConfig,
    pub identity_verifier: IdentityVerifierConfig,
    pub client_config: ClientConfig,
    pub enable_opentelemetry: bool,
}

impl SLIMAppConfig {
    /// Create a new SLIMAppConfig with SharedSecret authentication
    pub fn with_shared_secret(
        identity: impl Into<String>,
        endpoint: impl Into<String>,
        shared_secret: impl Into<String>,
    ) -> Self {
        let identity_str = identity.into();
        let secret = shared_secret.into();

        Self {
            identity: identity_str.clone(),
            identity_provider: IdentityProviderConfig::SharedSecret {
                id: identity_str.clone(),
                data: secret.clone(),
            },
            identity_verifier: IdentityVerifierConfig::SharedSecret {
                id: identity_str,
                data: secret,
            },
            client_config: slim_bindings::new_insecure_client_config(endpoint.into()),
            enable_opentelemetry: false,
        }
    }

    pub fn with_opentelemetry(mut self, enabled: bool) -> Self {
        self.enable_opentelemetry = enabled;
        self
    }
}

/// Split an ID into its components
/// Expected format: organization/namespace/application
pub fn split_id(id: &str) -> Result<Name> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() < 3 {
        return Err(SRPCError::InvalidId(format!(
            "ID must be in format organization/namespace/app, got: {}",
            id
        )));
    }

    Ok(Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0))
}

/// Create a local app with the given configuration and connect to SLIM service
/// Returns the bindings App and connection ID
pub async fn create_local_app(config: &SLIMAppConfig) -> Result<(std::sync::Arc<slim_bindings::App>, u64)> {
    use slim_bindings::{App as BindingsApp, Name as BindingsName, get_global_service, initialize_with_defaults};
    
    // Initialize crypto, runtime, global service (like Go's InitializeWithDefaults)
    initialize_with_defaults();
    
    // Parse identity to Name
    let app_name = split_id(&config.identity)?;
    let bindings_name = std::sync::Arc::new(BindingsName::from(&app_name));
    
    // Create app using async bindings API (avoids nested runtime issue)
    let app = BindingsApp::new_async(
        (*bindings_name).clone().into(),
        config.identity_provider.clone(),
        config.identity_verifier.clone(),
    ).await
    .map(std::sync::Arc::new)
    .map_err(|e| SRPCError::ParseIdentity(format!("App creation failed: {}", e)))?;
    
    // Connect to SLIM server (async to avoid nested runtime)
    let conn_id = get_global_service()
        .connect_async(config.client_config.clone())
        .await
        .map_err(|e| SRPCError::ParseIdentity(format!("Connection failed: {}", e)))?;
    
    // Subscribe to local name (async to avoid nested runtime)
    app.subscribe_async(app.name(), Some(conn_id))
        .await
        .map_err(|e| SRPCError::ParseIdentity(format!("Subscription failed: {}", e)))?;
    
    Ok((app, conn_id))
}
