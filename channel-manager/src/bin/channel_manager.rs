// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel Manager server binary.
//!
//! Connects to a SLIM node, creates channels from configuration, and exposes
//! a gRPC API for dynamic channel management.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agntcy_slim_channel_manager::config::Config;
use agntcy_slim_channel_manager::proto::channel_manager_service_server::ChannelManagerServiceServer;
use agntcy_slim_channel_manager::service::ChannelManagerServer;
use agntcy_slim_channel_manager::sessions::SessionsList;

use clap::Parser;
use slim_bindings::{
    Name, SessionConfig, SessionType, get_global_service, initialize_with_defaults,
    shutdown, IdentityProviderConfig, IdentityVerifierConfig,
};
use slim_bindings::ClientConfig as BindingsClientConfig;
use slim_tracing::TracingConfiguration;
use tokio::signal;
use tracing::{error, info};

/// Channel Manager - manages SLIM channels and participants
#[derive(Parser)]
#[command(name = "channel-manager")]
#[command(about = "Channel Manager for SLIM - manages channels and participants via gRPC")]
struct Args {
    /// Path to the configuration file
    #[arg(long = "config-file", default_value = "config.yaml")]
    config_file: PathBuf,
}

/// Create channels from the configuration file
async fn create_channels_from_config(
    app: &Arc<slim_bindings::App>,
    conn_id: u64,
    sessions: &Arc<SessionsList>,
    config: &Config,
) -> anyhow::Result<()> {
    for channel_cfg in &config.manager.channels {
        let channel_name =
            Name::from_string(channel_cfg.name.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;

        let session_config = SessionConfig {
            session_type: SessionType::Group,
            enable_mls: channel_cfg.mls_enabled,
            max_retries: Some(10),
            interval: Some(Duration::from_millis(1000)),
            metadata: std::collections::HashMap::new(),
        };

        let session = app
            .create_session_and_wait_async(session_config, Arc::new(channel_name))
            .await
            .map_err(|e| anyhow::anyhow!("failed to create session for {}: {e}", channel_cfg.name))?;

        for participant in &channel_cfg.participants {
            let participant_name =
                Name::from_string(participant.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;

            app.set_route_async(Arc::new(participant_name.clone()), conn_id)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to set route for {} in channel {}: {e}",
                        participant,
                        channel_cfg.name
                    )
                })?;

            session
                .invite_and_wait_async(Arc::new(participant_name))
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to invite {} to channel {}: {e}",
                        participant,
                        channel_cfg.name
                    )
                })?;
        }

        sessions
            .add_session(channel_cfg.name.clone(), session)
            .await?;

        info!(
            channel = %channel_cfg.name,
            participants = ?channel_cfg.participants,
            "Created channel and invited participants"
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with channel-manager module included
    let mut filters: Vec<String> = vec![
        "agntcy_slim_channel_manager".to_string(),
        "channel_manager".to_string(),
    ];
    // Add default slim modules
    filters.extend([
        "slim_datapath", "slim_service", "slim_controller", "slim_auth",
        "slim_config", "slim_mls", "slim_session", "slim_signal",
        "slim_tracing", "_slim_bindings", "slim",
    ].iter().map(|s| s.to_string()));
    let tracing = TracingConfiguration::default().with_filter(filters);
    let _guard = tracing.setup_tracing_subscriber()?;

    let args = Args::parse();

    // Load configuration
    let config = Config::load(&args.config_file)?;
    info!(config_file = %args.config_file.display(), "Configuration loaded");

    // Initialize SLIM (also sets up crypto provider; tracing subscriber already set so it reuses ours)
    initialize_with_defaults();
    let service = get_global_service();

    // Connect to the SLIM node using the full ClientConfig
    let client_config: BindingsClientConfig = config.manager.slim_connection.clone().into();
    let conn_id = service.connect_async(client_config).await.map_err(|e| {
        anyhow::anyhow!("failed to connect to SLIM node: {e}")
    })?;
    info!(
        endpoint = %config.manager.slim_connection.endpoint,
        conn_id,
        "Connected to SLIM node"
    );

    // Create the SLIM app with configured authentication
    let app_name = Arc::new(
        Name::from_string(config.manager.local_name.clone())
            .map_err(|e| anyhow::anyhow!("invalid local-name: {e}"))?,
    );
    let (core_provider, core_verifier) = config
        .manager
        .auth
        .to_identity_configs(&config.manager.local_name);
    let provider: IdentityProviderConfig = core_provider.into();
    let verifier: IdentityVerifierConfig = core_verifier.into();
    let app = service
        .create_app_async(app_name.clone(), provider, verifier)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create SLIM app: {e}"))?;

    // Subscribe to the local name
    app.subscribe_async(app_name, Some(conn_id))
        .await
        .map_err(|e| anyhow::anyhow!("failed to subscribe: {e}"))?;

    info!(
        local_name = %config.manager.local_name,
        "SLIM app created"
    );

    // Create sessions list
    let sessions = Arc::new(SessionsList::new());

    // Create channels from configuration
    if let Err(e) = create_channels_from_config(&app, conn_id, &sessions, &config).await {
        error!("Failed to create channels from config: {e}");
        return Err(e);
    }

    // Create gRPC server
    let server = ChannelManagerServer::new(app.clone(), conn_id, sessions.clone());
    let svc = ChannelManagerServiceServer::new(server);

    info!(
        endpoint = %config.manager.api_server.endpoint,
        "Starting gRPC server"
    );

    // Start gRPC server using ServerConfig (supports TLS, auth, keepalive, unix sockets, etc.)
    let server_future = config
        .manager
        .api_server
        .to_server_future(&[svc])
        .await
        .map_err(|e| anyhow::anyhow!("failed to create gRPC server: {e}"))?;

    // Run the server with graceful shutdown on Ctrl+C
    tokio::select! {
        result = server_future => {
            result.map_err(|e| anyhow::anyhow!("gRPC server error: {e}"))?;
        }
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received");
        }
    }

    // Cleanup
    info!("Shutting down...");
    sessions.delete_all(&app).await;
    shutdown().await.map_err(|e| anyhow::anyhow!("shutdown failed: {e}"))?;
    info!("Shutdown complete");

    Ok(())
}
