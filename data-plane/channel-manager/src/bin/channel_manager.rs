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
use slim_bindings::ClientConfig as BindingsClientConfig;
use slim_bindings::{
    IdentityProviderConfig, IdentityVerifierConfig, MlsSettings, Name, SessionConfig, SessionType,
    get_global_service, initialize_with_defaults, shutdown,
};
use slim_tracing::TracingConfiguration;
use tracing::{error, info, warn};

/// Channel Manager - manages SLIM channels and participants
#[derive(Parser)]
#[command(name = "channel-manager")]
#[command(about = "Channel Manager for SLIM")]
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
            mls_settings: if channel_cfg.mls_enabled {
                Some(MlsSettings::default())
            } else {
                None
            },
            max_retries: Some(10),
            interval: Some(Duration::from_millis(1000)),
            metadata: std::collections::HashMap::new(),
        };

        let session = app
            .create_session_and_wait_async(session_config, Arc::new(channel_name))
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to create session for {}: {e}", channel_cfg.name)
            })?;

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
    // Initialize logging with channel-manager modules included
    let filters: Vec<String> = vec![
        "agntcy_slim_channel_manager".to_string(),
        "channel_manager".to_string(),
    ];
    let tracing = TracingConfiguration::default().with_filter(filters);
    let _guard = tracing.setup_tracing_subscriber();

    let args = Args::parse();

    // Load configuration
    let config = Config::load(&args.config_file)?;
    info!(config_file = %args.config_file.display(), "Configuration loaded");

    // Initialize SLIM (also sets up crypto provider; tracing subscriber already set so it reuses ours)
    initialize_with_defaults();
    let service = get_global_service();

    // Connect to the SLIM node using the full ClientConfig
    let client_config: BindingsClientConfig = config.manager.slim_connection.clone().into();
    let conn_id = service
        .connect_async(client_config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect to SLIM node: {e}"))?;
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
        .create_app_with_direction_async(
            app_name.clone(),
            provider,
            verifier,
            slim_bindings::Direction::None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("failed to create SLIM app: {e}"))?;

    // Subscribe to the local name
    app.subscribe_async(app.name(), Some(conn_id))
        .await
        .map_err(|e| anyhow::anyhow!("failed to subscribe: {e}"))?;

    info!(
        local_name = %app.name(),
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

    // Run the server with graceful shutdown on SIGINT/SIGTERM
    tokio::select! {
        result = server_future => {
            result.map_err(|e| anyhow::anyhow!("gRPC server error: {e}"))?;
        }
        _ = slim_signal::shutdown() => {
            info!("Shutdown signal received");
        }
    }

    // Cleanup with timeout to avoid hanging on shutdown
    info!("Shutting down...");
    match tokio::time::timeout(Duration::from_secs(5), sessions.delete_all(&app)).await {
        Ok(()) => info!("All sessions cleaned up"),
        Err(_) => warn!("Session cleanup timed out, forcing shutdown"),
    }
    shutdown()
        .await
        .map_err(|e| anyhow::anyhow!("shutdown failed: {e}"))?;
    info!("Shutdown complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_args_parsing_default_config() {
        let args = Args::try_parse_from(["channel-manager"]);
        assert!(args.is_ok(), "parsing without arguments should succeed");
        let args = args.unwrap();
        assert_eq!(
            args.config_file,
            PathBuf::from("config.yaml"),
            "config_file should default to config.yaml"
        );
    }

    #[test]
    fn test_args_parsing_custom_config() {
        let args = Args::try_parse_from([
            "channel-manager",
            "--config-file",
            "/etc/channel-manager.yaml",
        ]);
        assert!(args.is_ok(), "parsing with custom config should succeed");
        let args = args.unwrap();
        assert_eq!(
            args.config_file,
            PathBuf::from("/etc/channel-manager.yaml"),
            "config_file should be set to provided path"
        );
    }

    #[test]
    fn test_args_parsing_invalid_flag() {
        let args = Args::try_parse_from(["channel-manager", "--invalid-flag"]);
        assert!(args.is_err(), "parsing with invalid flag should fail");
    }

    #[test]
    fn test_command_help() {
        let args = Args::try_parse_from(["channel-manager", "--help"]);
        // --help causes exit, so we expect an error, but it's not a real error
        assert!(args.is_err());
    }

    // This is a documentation test showing the expected config file format
    #[test]
    fn test_expected_config_file_format() {
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let config_content = r#"
manager:
  local_name: "channel-manager"
  slim_connection:
    endpoint: "http://localhost:10355"
  api_server:
    endpoint: "http://localhost:10356"
  channels:
    - name: "org/namespace/channel"
      mls_enabled: true
      participants:
        - "org/namespace/app1"
        - "org/namespace/app2"
"#;
        fs::write(temp_file.path(), config_content).expect("failed to write config");

        // Verify the file was written
        let content = fs::read_to_string(temp_file.path()).expect("failed to read");
        assert!(content.contains("channel-manager"));
        assert!(content.contains("mls_enabled: true"));
    }

    #[test]
    fn test_args_parsing_multiple_calls() {
        // Ensure Args parsing is deterministic
        let args1 = Args::try_parse_from(["channel-manager", "--config-file", "test.yaml"]);
        let args2 = Args::try_parse_from(["channel-manager", "--config-file", "test.yaml"]);

        assert!(args1.is_ok());
        assert!(args2.is_ok());
        assert_eq!(args1.unwrap().config_file, args2.unwrap().config_file);
    }
}
