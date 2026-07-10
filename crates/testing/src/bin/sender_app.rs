// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SLIM Sender App
//!
//! This app demonstrates an active sender that:
//! 1. Creates a SLIM application with specified identity and shared secret
//! 2. Creates a session (point-to-point or group)
//! 3. Sends messages at regular intervals
//! 4. Receives and validates replies from participants
//!
//! Usage:
//! ```bash
//! # Point-to-point
//! cargo run --bin sender-app -- \
//!   -l agntcy/ns/bob \
//!   -t p2p \
//!   -p agntcy/ns/alice
//!
//! # Group (with custom group name and secret)
//! cargo run --bin sender-app -- \
//!   -l agntcy/ns/bob \
//!   -k a-very-long-shared-secret-abcdef1234567890 \
//!   -t group \
//!   -g agntcy/slim/my-channel \
//!   -p agntcy/ns/alice agntcy/ns/charlie \
//!   -n 20 -i 200
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::client::ClientConfig;
use slim_config::component::ComponentBuilder;
use slim_config::tls::client::TlsClientConfig;
use slim_datapath::api::{ProtoName, ProtoPublishType, ProtoSessionType};
use slim_service::ServiceBuilder;
use slim_session::{Direction, SessionConfig, session_config::MlsSettings};
use slim_tracing::TracingConfiguration;

/// Print with timestamp prefix
macro_rules! tprintln {
    ($($arg:tt)*) => {
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % 86400;
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            let s = secs % 60;
            let millis = now.subsec_millis();
            eprintln!("{:02}:{:02}:{:02}.{:03}  {}", hours, mins, s, millis, format_args!($($arg)*));
        }
    };
}

/// Command-line arguments for the sender application
const DEFAULT_SECRET: &str = "1234567890-1234567890-1234567890-123456";

#[derive(Parser, Debug)]
struct Args {
    /// Local identity in format: organization/namespace/application
    #[arg(short, long, required = true)]
    local: String,

    /// Shared secret for authentication (min 32 bytes)
    #[arg(short = 'k', long, default_value = DEFAULT_SECRET)]
    shared_secret: String,

    /// SLIM control plane endpoint
    #[arg(short, long, default_value = "http://localhost:46357")]
    slim: String,

    /// Session type: "p2p" or "group"
    #[arg(short = 't', long, required = true)]
    session_type: String,

    /// Group session name (only used for group sessions)
    #[arg(short, long, default_value = "agntcy/slim/test-app-channel")]
    group_name: String,

    /// List of participant identities (format: organization/namespace/application)
    /// For p2p: exactly 1 participant
    /// For group: at least 1 participant
    #[arg(short, long, required = true, num_args = 1..)]
    participants: Vec<String>,

    /// Number of messages to send
    #[arg(short = 'n', long, default_value = "10")]
    count: usize,

    /// Interval between messages in milliseconds
    #[arg(short, long, default_value = "100")]
    interval_ms: u64,
}

/// Parse session type from string
fn parse_session_type(s: &str) -> Result<ProtoSessionType> {
    match s.to_lowercase().as_str() {
        "p2p" | "point-to-point" | "pointtopoint" => Ok(ProtoSessionType::PointToPoint),
        "group" | "multicast" => Ok(ProtoSessionType::Multicast),
        _ => bail!("Invalid session type '{}'. Expected 'p2p' or 'group'", s),
    }
}

/// Main sender loop
async fn run_sender(args: Args) -> Result<()> {
    let tracing = TracingConfiguration::default();
    let _guard = tracing
        .setup_tracing_subscriber()
        .context("failed to setup tracing")?;

    slim_config::tls::provider::initialize_crypto_provider();

    // Validate arguments
    let session_type = parse_session_type(&args.session_type)?;

    if session_type == ProtoSessionType::PointToPoint && args.participants.len() != 1 {
        bail!("Point-to-point sessions require exactly 1 participant");
    }

    if args.participants.is_empty() {
        bail!("At least 1 participant is required");
    }

    // Parse the local identity and participants
    let local_name =
        ProtoName::parse_name(&args.local).map_err(|e| anyhow!("invalid local name: {e}"))?;
    let participant_names: Vec<ProtoName> = args
        .participants
        .iter()
        .map(|s| ProtoName::parse_name(s).map_err(|e| anyhow!("invalid participant name: {e}")))
        .collect::<Result<_>>()?;

    // Create service and connect
    let service = Arc::new(
        ServiceBuilder::new()
            .build("sender-app".to_string())
            .context("failed to create SLIM service")?,
    );

    let client_config =
        ClientConfig::with_endpoint(&args.slim).with_tls_setting(TlsClientConfig::insecure());
    let conn_id = service
        .connect(&client_config)
        .await
        .context("connect to server failed")?;

    tprintln!("Connected to control plane with connection ID: {}", conn_id);

    // Create the app with shared secret auth
    let name_str = local_name.to_string();
    let mut provider = AuthProvider::shared_secret_from_str(&name_str, &args.shared_secret)
        .context("failed to create auth provider")?;
    let mut verifier = AuthVerifier::shared_secret_from_str(&name_str, &args.shared_secret)
        .context("failed to create auth verifier")?;
    provider
        .initialize()
        .await
        .map_err(|e| anyhow!("provider init failed: {e}"))?;
    verifier
        .initialize()
        .await
        .map_err(|e| anyhow!("verifier init failed: {e}"))?;
    let (app, _notification_rx) = service
        .create_app_with_direction(&local_name, provider, verifier, Direction::Bidirectional)
        .context("failed to create app")?;

    let full_name = app.app_name().to_string();
    tprintln!("[{}] Application created", full_name);

    // Subscribe to local name
    app.subscribe(&local_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    // For p2p, destination is the single participant
    // For group, destination is the group name (configurable via --group-name)
    let destination = if session_type == ProtoSessionType::Multicast {
        ProtoName::parse_name(&args.group_name).map_err(|e| anyhow!("invalid group name: {e}"))?
    } else {
        participant_names[0].clone()
    };

    tprintln!(
        "[{}] Creating {:?} session with destination {}...",
        full_name,
        session_type,
        destination
    );

    // Set routes for all participants
    for participant in &participant_names {
        tprintln!("[{}] Setting route for {}", full_name, participant);
        app.set_route(participant, conn_id)
            .await
            .context("set route failed")?;
    }

    // Create session with MLS always enabled
    let session_config = SessionConfig {
        session_type,
        mls_settings: Some(MlsSettings::default()),
        max_retries: Some(5),
        interval: Some(Duration::from_secs(1)),
        initiator: true,
        metadata: HashMap::new(),
    };

    let (session_ctx, completion) = app
        .create_session(session_config, destination.clone(), None)
        .await
        .context("failed to create session")?;

    // Wait for session establishment
    tokio::time::timeout(Duration::from_secs(35), completion)
        .await
        .context("session establishment timed out")?
        .context("session establishment failed")?;

    let (weak_session, mut session_rx) = session_ctx.into_parts();
    let controller = weak_session
        .upgrade()
        .ok_or_else(|| anyhow!("session closed"))?;
    let session_id = controller.id();

    tprintln!("[{}] Session {} established", full_name, session_id);

    // For group sessions, invite all participants
    if session_type == ProtoSessionType::Multicast {
        for participant in &participant_names {
            tprintln!("[{}] Inviting {} to session...", full_name, participant);
            controller
                .invite_participant(participant)
                .await
                .context("invite failed")?
                .await
                .context("invite completion failed")?;
            tprintln!("[{}] {} joined session", full_name, participant);
        }
    }

    let interval = Duration::from_millis(args.interval_ms);
    let expected_replies_per_message = participant_names.len();
    let total_expected_replies = args.count * expected_replies_per_message;

    tprintln!(
        "[{}] Sending {} messages with {}ms interval...",
        full_name,
        args.count,
        args.interval_ms
    );

    // Spawn a background task to collect replies
    let full_name_clone = full_name.clone();
    let participants_clone = participant_names.clone();
    let reply_task = tokio::spawn(async move {
        let mut total_replies_received = 0;
        let recv_timeout = Duration::from_secs(10);

        loop {
            match tokio::time::timeout(recv_timeout, session_rx.recv()).await {
                Ok(Some(Ok(msg))) => {
                    let payload = match msg.message_type.as_ref() {
                        Some(ProtoPublishType(publish)) => publish
                            .msg
                            .as_ref()
                            .and_then(|content| content.as_application_payload().ok())
                            .map(|p| p.blob.clone())
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    let source = msg.get_source();
                    let payload_str = String::from_utf8_lossy(&payload);

                    // Only count replies from known participants (ignore echoed messages)
                    let is_from_participant =
                        participants_clone.iter().any(|p| source.match_prefix(p));

                    if !is_from_participant {
                        tprintln!(
                            "[{}] Ignoring message from non-participant {}: {}",
                            full_name_clone,
                            source,
                            payload_str
                        );
                        continue;
                    }

                    tprintln!(
                        "[{}] Reply from {}: {}",
                        full_name_clone,
                        source,
                        payload_str
                    );

                    total_replies_received += 1;
                    if total_replies_received >= total_expected_replies {
                        break;
                    }
                }
                Ok(Some(Err(e))) => {
                    tprintln!("[{}] Error receiving reply: {}", full_name_clone, e);
                    break;
                }
                Ok(None) => {
                    tprintln!("[{}] Session channel closed", full_name_clone);
                    break;
                }
                Err(_) => continue, // timeout, keep waiting
            }
        }

        total_replies_received
    });

    // Send messages at fixed intervals
    for i in 0..args.count {
        let message = format!("Message {}", i + 1);
        tprintln!("[{}] Sending: {}", full_name, message);

        if let Err(e) = controller
            .publish(controller.dst(), message.into_bytes(), None, None)
            .await
            .context("publish failed")
        {
            tprintln!("[{}] Error sending message: {}", full_name, e);
        }

        // After 10 messages, pause and resume
        if i + 1 == 10 {
            tprintln!("[{}] Pausing session after 10 messages...", full_name);
            let handle = controller.pause().await.context("pause failed")?;
            handle.await.context("pause completion failed")?;
            tprintln!("[{}] Paused. Waiting 10 seconds...", full_name);
            tokio::time::sleep(Duration::from_secs(10)).await;
            tprintln!("[{}] Resuming session...", full_name);
            let handle = controller.resume().await.context("resume failed")?;
            handle.await.context("resume completion failed")?;
            tprintln!("[{}] Resumed.", full_name);
        }

        tokio::time::sleep(interval).await;
    }

    tprintln!(
        "[{}] Finished sending messages, waiting for remaining replies...",
        full_name
    );

    // Wait for reply collection task to finish or timeout
    let total_replies_received = tokio::select! {
        result = reply_task => {
            match result {
                Ok(data) => data,
                Err(e) => {
                    tprintln!("[{}] Reply collection task error: {}", full_name, e);
                    0
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(30)) => {
            tprintln!("[{}] Timeout waiting for replies", full_name);
            0
        }
    };

    // Summary
    tprintln!("\n[{}] === Summary ===", full_name);
    tprintln!(
        "[{}] Replies received: {}/{}",
        full_name,
        total_replies_received,
        total_expected_replies
    );

    if total_replies_received == total_expected_replies {
        tprintln!("[{}] ✓ All participants replied correctly", full_name);
    } else {
        tprintln!(
            "[{}] ✗ Missing {} replies",
            full_name,
            total_expected_replies - total_replies_received
        );
    }

    // Delete session
    tprintln!("[{}] Deleting session...", full_name);
    app.delete_session(&controller)
        .context("delete session failed")?
        .await
        .context("delete session completion failed")?;

    tprintln!("[{}] Sender stopped", full_name);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    run_sender(args).await
}
