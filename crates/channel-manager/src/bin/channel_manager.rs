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

use anyhow::Context;
use clap::Parser;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::component::ComponentBuilder;
use slim_datapath::api::{ProtoName, ProtoSessionType};
use slim_persistence::{MlsEncryptionKey, PersistenceConfig};
use slim_service::app::App;
use slim_session::{Direction, SessionConfig, session_config::MlsSettings};
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

/// Reconcile channels defined in the config file against the live session state.
///
/// For each channel in the config:
/// - If restored from DB: perform a full roster reconciliation against the config —
///   remove participants no longer in the config, invite participants not yet in the
///   session. The channel-manager's own identity is always in the roster and is skipped.
/// - If freshly created: invite all configured participants; any error is fatal.
async fn create_channels_from_config(
    app: &Arc<App<AuthProvider, AuthVerifier>>,
    conn_id: u64,
    sessions: &Arc<SessionsList>,
    config: &Config,
) -> anyhow::Result<()> {
    // 3-component key of the channel-manager itself — always present in every roster,
    // never a user-configured participant, so excluded from all reconciliation checks.
    let self_key = {
        let n = app.app_name();
        let (c0, c1, c2) = n.str_components();
        format!("{c0}/{c1}/{c2}")
    };

    for channel_cfg in &config.manager.channels {
        let (controller, restored) = if let Some(ctrl) =
            sessions.get_session(&channel_cfg.name).await
        {
            info!(channel = %channel_cfg.name, "Channel restored from DB; reconciling participants");
            (ctrl, true)
        } else {
            let channel_name =
                ProtoName::parse_name(&channel_cfg.name).map_err(|e| anyhow::anyhow!("{e}"))?;

            let session_config = SessionConfig {
                session_type: ProtoSessionType::Multicast,
                mls_settings: if channel_cfg.mls_enabled {
                    Some(MlsSettings::default())
                } else {
                    None
                },
                max_retries: Some(10),
                interval: Some(Duration::from_millis(1000)),
                initiator: true,
                metadata: std::collections::HashMap::new(),
            };

            let (session, completion) = app
                .create_session(session_config, channel_name, None)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("failed to create session for {}: {e}", channel_cfg.name)
                })?;
            completion.await.map_err(|e| {
                anyhow::anyhow!("session creation failed for {}: {e}", channel_cfg.name)
            })?;

            let ctrl = session.session_arc().ok_or_else(|| {
                anyhow::anyhow!("session already closed for {}", channel_cfg.name)
            })?;

            sessions
                .add_session(channel_cfg.name.clone(), session)
                .await?;

            info!(channel = %channel_cfg.name, "Created channel");
            (ctrl, false)
        };

        // Build the set of 3-component names declared in the config for this channel.
        let config_set: std::collections::HashSet<String> = channel_cfg
            .participants
            .iter()
            .filter_map(|p| ProtoName::parse_name(p).ok())
            .map(|n| {
                let (c0, c1, c2) = n.str_components();
                format!("{c0}/{c1}/{c2}")
            })
            .collect();

        // For restored sessions: build the roster map and reconcile.
        // roster_set holds the 3-component keys of participants already in the session
        // (excluding the channel-manager itself) so we can skip redundant invites.
        let roster_set: std::collections::HashSet<String> = if restored {
            let roster = controller.participants_list().await.unwrap_or_default();

            // Remove participants no longer in the config (skip self).
            for (name, _) in &roster {
                let (c0, c1, c2) = name.str_components();
                let key = format!("{c0}/{c1}/{c2}");
                if key == self_key || config_set.contains(&key) {
                    continue;
                }
                info!(channel = %channel_cfg.name, participant = %name, "Removing participant no longer in config");
                match controller.remove_participant(name).await {
                    Ok(completion) => {
                        if let Err(e) = completion.await {
                            warn!(channel = %channel_cfg.name, participant = %name, "Failed to remove participant: {e}");
                        }
                    }
                    Err(e) => {
                        warn!(channel = %channel_cfg.name, participant = %name, "Failed to remove participant: {e}");
                    }
                }
            }

            roster
                .into_iter()
                .filter_map(|(name, _)| {
                    let (c0, c1, c2) = name.str_components();
                    let key = format!("{c0}/{c1}/{c2}");
                    if key == self_key { None } else { Some(key) }
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        // Invite participants declared in config that are not already in the session.
        for participant in &channel_cfg.participants {
            let participant_name =
                ProtoName::parse_name(participant).map_err(|e| anyhow::anyhow!("{e}"))?;

            let (c0, c1, c2) = participant_name.str_components();
            let key = format!("{c0}/{c1}/{c2}");
            if roster_set.contains(&key) {
                tracing::debug!(channel = %channel_cfg.name, participant = %participant, "Participant already in session, skipping invite");
                continue;
            }

            app.set_route(&participant_name, conn_id)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to set route for {} in channel {}: {e}",
                        participant,
                        channel_cfg.name
                    )
                })?;

            let completion = controller
                .invite_participant(&participant_name)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to invite {} to channel {}: {e}",
                        participant,
                        channel_cfg.name
                    )
                })?;
            completion.await.map_err(|e| {
                anyhow::anyhow!(
                    "failed to invite {} to channel {}: {e}",
                    participant,
                    channel_cfg.name
                )
            })?;
        }

        info!(
            channel = %channel_cfg.name,
            participants = ?channel_cfg.participants,
            "Channel ready"
        );
    }

    // Delete sessions that exist in the DB but are no longer in the config.
    // Config is the source of truth in config mode.
    let config_channels: std::collections::HashSet<&str> = config
        .manager
        .channels
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    for name in sessions.list_channel_names().await {
        if !config_channels.contains(name.as_str()) {
            info!(channel = %name, "Channel removed from config; deleting session");
            if let Err(e) = sessions.remove_session(&name, app).await {
                warn!(channel = %name, "Failed to delete removed channel: {e}");
            }
        }
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

    // Initialize crypto provider (required before any TLS operations)
    slim_config::tls::provider::initialize_crypto_provider();

    // Create a SLIM service
    let service = slim_service::ServiceBuilder::new()
        .build("channel-manager-service".to_string())
        .map_err(|e| anyhow::anyhow!("failed to create SLIM service: {e}"))?;

    // Connect to the SLIM node
    let conn_id = service
        .connect(&config.manager.slim_connection)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect to SLIM node: {e}"))?;

    info!(
        endpoint = %config.manager.slim_connection.endpoint,
        conn_id,
        "Connected to SLIM node"
    );

    // Create the SLIM app with configured authentication
    let app_name = ProtoName::parse_name(&config.manager.local_name)
        .map_err(|e| anyhow::anyhow!("invalid local-name: {e}"))?;
    let (core_provider, core_verifier) = config
        .manager
        .auth
        .to_identity_configs(&config.manager.local_name);

    let mut provider = core_provider
        .build_auth_provider()
        .context("failed to build auth provider")?;
    let mut verifier = core_verifier
        .build_auth_verifier()
        .context("failed to build auth verifier")?;

    // Initialize auth (required before use)
    provider
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("failed to initialize identity provider: {e}"))?;
    verifier
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("failed to initialize identity verifier: {e}"))?;

    let persistence = config
        .manager
        .persistence
        .as_ref()
        .map(|p| match &p.encryption_passphrase {
            Some(passphrase) => PersistenceConfig::with_key(
                &p.path,
                MlsEncryptionKey::Passphrase(passphrase.clone()),
            ),
            None => PersistenceConfig::new(&p.path),
        });

    let (app, _rx) = service.create_app_with_direction_and_persistence(
        &app_name,
        provider,
        verifier,
        Direction::None,
        persistence,
    )?;

    // Subscribe to the local name
    app.subscribe(app.app_name(), Some(conn_id))
        .await
        .map_err(|e| anyhow::anyhow!("failed to subscribe: {e}"))?;

    info!(
        local_name = %app.app_name(),
        "SLIM app created"
    );

    // Create sessions list
    let sessions = Arc::new(SessionsList::new());
    let arc_app = Arc::new(app);

    // Restore sessions persisted by a previous run (no-op when persistence is disabled).
    match arc_app.restore_sessions(conn_id).await {
        Ok(restored) => {
            info!(
                count = restored.len(),
                "Sessions found in persistent storage"
            );
            for ctx in restored {
                match ctx.session_arc() {
                    None => warn!("Restored session context has no live session arc; skipping"),
                    Some(session) => {
                        let dst = session.dst();
                        if dst.str_name.is_none() {
                            warn!("Skipping restored session with no string name in destination");
                            continue;
                        }
                        let (c0, c1, c2) = dst.str_components();
                        let channel_name = format!("{c0}/{c1}/{c2}");
                        if let Err(e) = sessions.add_session(channel_name.clone(), ctx).await {
                            warn!(channel = %channel_name, "failed to restore session: {e}");
                        } else {
                            info!(channel = %channel_name, "Restored session from persistent storage");
                        }
                    }
                }
            }
        }
        Err(e) => warn!("Failed to restore sessions: {e}"),
    }

    // Config mode: channels section is non-empty → config is the source of truth.
    // API mode:    channels section is empty → DB (restored above) is the source of truth.
    let config_mode = !config.manager.channels.is_empty();

    if config_mode
        && let Err(e) = create_channels_from_config(&arc_app, conn_id, &sessions, &config).await
    {
        error!("Failed to reconcile channels from config: {e}");
        return Err(e);
    }

    // Create gRPC server
    let server = ChannelManagerServer::new(arc_app.clone(), conn_id, sessions.clone(), config_mode);
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

    // Cleanup: either delete sessions from SLIM or just go offline, depending on config.
    info!("Shutting down...");
    let delete_on_shutdown = config
        .manager
        .persistence
        .as_ref()
        .map(|p| p.delete_sessions_on_shutdown)
        .unwrap_or(true);
    if delete_on_shutdown {
        sessions.delete_all(&arc_app).await;
        info!("All sessions deleted");
    } else {
        sessions.close_all().await;
        info!("All sessions closed");
    }
    service
        .shutdown()
        .await
        .map_err(|e| anyhow::anyhow!("service shutdown failed: {e}"))?;
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
