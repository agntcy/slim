// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SLIM Receiver App
//!
//! This app demonstrates a passive listener that:
//! 1. Creates a SLIM application with specified identity and shared secret
//! 2. Waits for an incoming session
//! 3. Receives messages from that session
//! 4. Replies to each message using publish_to
//!
//! Usage:
//! ```bash
//! cargo run --bin receiver-app -- \
//!   -l agntcy/ns/alice \
//!   -k a-very-long-shared-secret-abcdef1234567890
//! ```

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::client::ClientConfig;
use slim_config::component::ComponentBuilder;
use slim_config::tls::client::TlsClientConfig;
use slim_datapath::api::{ProtoName, ProtoPublishType};
use slim_service::ServiceBuilder;
use slim_session::{Direction, Notification};
use slim_tracing::TracingConfiguration;
use tokio::signal;

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

/// Command-line arguments for the receiver application
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
}

/// Main receiver loop
async fn run_receiver(args: Args) -> Result<()> {
    let tracing = TracingConfiguration::default();
    let _guard = tracing
        .setup_tracing_subscriber()
        .context("failed to setup tracing")?;

    slim_config::tls::provider::initialize_crypto_provider();

    // Parse the local identity
    let local_name =
        ProtoName::parse_name(&args.local).map_err(|e| anyhow!("invalid name: {e}"))?;

    // Create service and connect
    let service = Arc::new(
        ServiceBuilder::new()
            .build("receiver-app".to_string())
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
    let (app, mut notification_rx) = service
        .create_app_with_direction(&local_name, provider, verifier, Direction::Bidirectional)
        .context("failed to create app")?;

    let full_name = app.app_name().to_string();

    // Subscribe to local name
    app.subscribe(&local_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    tprintln!("[{}] Waiting for incoming session...", full_name);

    // Wait for an incoming session notification
    let session_ctx = loop {
        match notification_rx.recv().await {
            Some(Ok(Notification::NewSession(ctx))) => break ctx,
            Some(Ok(Notification::NewMessage(_))) => continue,
            Some(Err(e)) => {
                tprintln!("[{}] notification error: {} (retrying)", full_name, e);
                continue;
            }
            None => anyhow::bail!("notification channel closed"),
        }
    };

    let controller = session_ctx
        .session_arc()
        .ok_or_else(|| anyhow!("session closed"))?;
    let mut session_rx = session_ctx.rx;
    let session_id = controller.id();

    tprintln!(
        "[{}] New session {} established from {}",
        full_name,
        session_id,
        controller.dst()
    );

    // Track the session initiator — only reply to messages from them
    let mut initiator: Option<ProtoName> = None;
    let mut message_count: u32 = 0;

    // Loop to receive messages and reply
    let recv_timeout = Duration::from_secs(5);
    loop {
        tokio::select! {
            result = tokio::time::timeout(recv_timeout, session_rx.recv()) => {
                match result {
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
                        let input_conn = msg.get_incoming_conn();
                        let payload_str = String::from_utf8_lossy(&payload);

                        // Set initiator from first message received
                        if initiator.is_none() {
                            initiator = Some(source.clone());
                        }

                        // Only reply to messages from the session initiator
                        if initiator.as_ref() != Some(&source) {
                            tprintln!(
                                "[{}] Ignoring message from non-initiator {}: {}",
                                full_name, source, payload_str
                            );
                            continue;
                        }

                        // if message id = 10 and local name is b/b/b do controller.pause
                        // wait for 10 seconds and then controller.resume
                        message_count += 1;
                        if message_count == 10 && local_name.to_string() == "b/b/b/NULL_COMPONENT" {
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

                        tprintln!(
                            "[{}] Received message from initiator {}: {}",
                            full_name, source, payload_str
                        );

                        // Reply to the sender using publish_to (fire-and-forget, don't wait for ACK)
                        let reply = payload_str.to_string();
                        controller
                            .publish_to(&source, input_conn, reply.into_bytes(), None, None)
                            .await
                            .context("publish_to failed")?;

                        tprintln!("[{}] Reply to initiator {}", full_name, source);

                        // if local name is b/b/b and the received message
                    }
                    Ok(Some(Err(e))) => {
                        tprintln!("[{}] Error receiving message: {}", full_name, e);
                        break;
                    }
                    Ok(None) => {
                        tprintln!("[{}] Session channel closed", full_name);
                        break;
                    }
                    Err(_) => {
                        // Timeout — continue waiting
                        continue;
                    }
                }
            },
            _ = signal::ctrl_c() => {
                tprintln!("\n[{}] Received Ctrl+C, shutting down gracefully...", full_name);
                break;
            }
        }
    }

    tprintln!("[{}] Receiver stopped", full_name);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    run_receiver(args).await
}
