// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::app::App;
use slim_service::Service;
use slim_session::{Notification, SessionError};
use std::collections::HashMap;
use tokio::sync::mpsc::Receiver;

pub const DEADLINE_KEY: &str = "slimrpc-timeout";
pub const MAX_TIMEOUT: u64 = 36000; // 10h in seconds

#[derive(Debug, Clone)]
pub struct SLIMAppConfig {
    pub identity: String,
    pub slim_client_config: HashMap<String, String>,
    pub enable_opentelemetry: bool,
    pub shared_secret: String,
}

impl SLIMAppConfig {
    pub fn new(
        identity: impl Into<String>,
        endpoint: impl Into<String>,
        shared_secret: impl Into<String>,
    ) -> Self {
        let mut config = HashMap::new();
        config.insert("endpoint".to_string(), endpoint.into());
        
        Self {
            identity: identity.into(),
            slim_client_config: config,
            enable_opentelemetry: false,
            shared_secret: shared_secret.into(),
        }
    }

    pub fn with_opentelemetry(mut self, enabled: bool) -> Self {
        self.enable_opentelemetry = enabled;
        self
    }

    pub fn identity_name(&self) -> Result<Name> {
        split_id(&self.identity)
    }
}

/// Split an ID into its components
/// Expected format: organization/namespace/application
pub fn split_id(id: &str) -> Result<Name> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() < 3 {
        anyhow::bail!("ID must be in format organization/namespace/app, got: {}", id);
    }

    Ok(Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0))
}

/// Convert a service/method to a subscription Name
pub fn service_and_method_to_name(
    base_name: &Name,
    service_method: &str,
) -> Result<Name> {
    let parts: Vec<&str> = service_method.split('/').collect();
    if parts.len() < 3 {
        anyhow::bail!("Service method must be in format /service/method");
    }

    let service_name = parts[1];
    let method_name = parts[2];
    
    method_to_name(base_name, service_name, method_name)
}

/// Convert service and method names to a subscription Name
pub fn method_to_name(
    base_name: &Name,
    service_name: &str,
    method_name: &str,
) -> Result<Name> {
    let components = base_name.to_string();
    let parts: Vec<&str> = components.split('/').collect();
    
    if parts.len() < 3 {
        anyhow::bail!("Base name must have at least 3 components");
    }

    let subscription_name = format!("{}-{}-{}", parts[2], service_name, method_name);
    
    Ok(Name::from_strings([parts[0], parts[1], &subscription_name]))
}

/// Create shared secret identity provider and verifier
pub fn create_shared_secret_auth(
    identity: &str,
    secret: &str,
) -> (SharedSecret, SharedSecret) {
    let provider = SharedSecret::new(identity, secret);
    let verifier = SharedSecret::new(identity, secret);
    (provider, verifier)
}

/// Create a local app with the given configuration and connect to SLIM service
/// Returns the app, notification receiver, and connection ID
pub async fn create_local_app(
    config: &SLIMAppConfig,
    service: &Service,
) -> Result<(
    App<SharedSecret, SharedSecret>,
    Receiver<std::result::Result<Notification, SessionError>>,
    u64,
)> {
    // Initialize rustls crypto provider (required for TLS operations)
    slim_config::tls::provider::initialize_crypto_provider();

    // Parse identity
    let local_name = config.identity_name()
        .context("Failed to parse identity")?;

    // Create shared secret auth
    let (provider, verifier) = create_shared_secret_auth(&config.identity, &config.shared_secret);

    // Create app
    let (app, rx) = service
        .create_app(&local_name, provider, verifier)
        .context("Failed to create app")?;

    // Connect to SLIM service
    let endpoint = config
        .slim_client_config
        .get("endpoint")
        .context("Missing endpoint in config")?;
    
    // Create ClientConfig from endpoint
    let client_config = slim_config::grpc::client::ClientConfig::with_endpoint(endpoint);
    
    let conn_id = service
        .connect(&client_config)
        .await
        .context("Failed to connect to SLIM service")?;

    // Subscribe to the local name
    app.subscribe(&local_name, Some(conn_id))
        .await
        .context("Failed to subscribe to local name")?;

    Ok((app, rx, conn_id))
}
