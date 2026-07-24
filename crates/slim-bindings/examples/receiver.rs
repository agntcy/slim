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
//! ## Modes
//!
//! **Config-based (recommended):** Supply `--slim-config` with a path to a `slim.yaml`
//! file, or omit it to use hierarchical discovery (walks up from CWD, then
//! `~/.slim/config.yaml`).
//!
//! ```bash
//! cargo run --example receiver -- --slim-config /path/to/slim.yaml
//! cargo run --example receiver  # discovery mode — needs slim.yaml in CWD or above
//! ```
//!
//! **Manual:** Supply `--local` and `--shared-secret` (legacy, backward-compatible).
//!
//! ```bash
//! cargo run --example receiver -- \
//!   --local agntcy/ns/alice \
//!   --shared-secret a-very-long-shared-secret-abcdef1234567890
//! ```
//!
//! ## Rejoin mode
//!
//! After receiving some messages, press Ctrl+C to shut down the receiver. The
//! MLS session state is persisted (requires config mode so the cache directory
//! is known). Restart with `--rejoin` to restore the session and continue:
//!
//! ```bash
//! # First run — join and receive
//! cargo run --example receiver -- --slim-config /path/to/slim.yaml
//! # ... Ctrl+C to stop ...
//!
//! # Second run — rejoin the persisted session
//! cargo run --example receiver -- --slim-config /path/to/slim.yaml --rejoin
//! ```

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use slim_bindings::{
    Name, get_global_service, initialize_with_defaults, load_slim_config,
    new_insecure_client_config, shutdown,
};
use tokio::signal;

/// Command-line arguments for the receiver application
#[derive(Parser, Debug)]
struct Args {
    /// Path to a slim.yaml config file. When absent the receiver searches
    /// upward from the current directory (then ~/.slim/config.yaml).
    /// Mutually exclusive with --local / --shared-secret.
    #[arg(long)]
    slim_config: Option<String>,

    /// Local identity in format: organization/namespace/application (manual mode)
    #[arg(long)]
    local: Option<String>,

    /// Shared secret for authentication (manual mode)
    #[arg(long)]
    shared_secret: Option<String>,

    /// SLIM node endpoint (manual mode only, ignored when --slim-config is used)
    #[arg(long, default_value = "http://localhost:46357")]
    slim: String,

    /// Rejoin a previously persisted group session instead of waiting for a
    /// new invitation.
    ///
    /// Requires config mode (slim.yaml provides a stable cache directory for
    /// MLS state persistence). The receiver must have joined the session in a
    /// prior run before `--rejoin` can be used.
    #[arg(long, default_value = "false")]
    rejoin: bool,
}

/// Parse a name string in format "org/namespace/app" into a Name object
fn parse_name(id: &str) -> Result<Name, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() != 3 {
        return Err(format!(
            "Invalid name format '{id}'. Expected format: organization/namespace/application"
        )
        .into());
    }

    Ok(Name::new(
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

/// Main receiver loop
async fn run_receiver(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize SLIM with default configuration
    initialize_with_defaults();

    let service = get_global_service();

    // --rejoin requires config mode for stable identity + persistent cache dir.
    if args.rejoin && (args.local.is_some() || args.shared_secret.is_some()) {
        println!(
            "Warning: --rejoin requires config mode. Ignoring --local / --shared-secret."
        );
    }

    // Determine which mode to use.
    // --rejoin forces config mode; otherwise the existing heuristic applies.
    let use_slim_config = args.rejoin
        || args.slim_config.is_some()
        || (args.local.is_none() && args.shared_secret.is_none());

    let (app, full_name, conn_id) = if use_slim_config {
        // ── Config-based mode ─────────────────────────────────────────────
        // load_slim_config discovers slim.yaml (or uses the explicit path),
        // applies env-var overrides, then create_app_from_slim_config does
        // connect + create_app + subscribe in one call.
        let config = load_slim_config(args.slim_config.clone()).map_err(|e| {
            format!(
                "Failed to load SLIM config{}: {e}",
                args.slim_config
                    .as_deref()
                    .map(|p| format!(" from '{p}'"))
                    .unwrap_or_default()
            )
        })?;

        let handle = service.create_app_from_slim_config_async(config).await?;
        let full_name = handle.name.to_string();
        let conn_id = handle.conn_id;
        println!("Connected via slim.yaml (app: {full_name}, conn: {conn_id})");
        (handle.app, full_name, conn_id)
    } else {
        // ── Manual mode (legacy) ──────────────────────────────────────────
        let local = args.local.ok_or("--local is required in manual mode")?;
        let secret = args.shared_secret.ok_or("--shared-secret is required in manual mode")?;

        let local_name = parse_name(&local)?;
        let local_name_arc = Arc::new(local_name);

        let client_config = new_insecure_client_config(args.slim);
        let conn_id = service.connect_async(client_config).await?;

        println!("Connected to control plane with connection ID: {conn_id}");

        let app = service
            .create_app_with_secret_async(local_name_arc.clone(), secret)
            .await?;

        let full_name = app.name().to_string();

        app.subscribe_async(local_name_arc, Some(conn_id)).await?;
        (app, full_name, conn_id)
    };

    // Obtain the active session — either by restoring a persisted one or by
    // waiting for a new invitation from the sender.
    let session = if args.rejoin {
        println!("[{full_name}] Restoring persisted session...");

        let mut sessions = app.restore_sessions_async(conn_id).await?;
        let session = sessions.pop().ok_or(
            "No persisted session found. Run without --rejoin first to join a group session.",
        )?;

        let session_id = session.session_id()?;
        println!("[{full_name}] Rejoined session {session_id}");
        session
    } else {
        println!("[{full_name}] Waiting for incoming session...");

        let session = app.listen_for_session_async(None).await?;

        let session_id = session.session_id()?;
        let destination = session.destination()?;
        println!("[{full_name}] New session {session_id} established from {destination}");
        session
    };

    // Loop to receive messages and reply
    loop {
        tokio::select! {
            result = session.get_message_async(Some(Duration::from_secs(5))) => {
                match result {
                    Ok(received_msg) => {
                        let payload = String::from_utf8_lossy(&received_msg.payload);
                        let source = &received_msg.context.source_name;

                        println!(
                            "[{full_name}] Received from {source}: {payload}"
                        );

                        // Reply to the sender using publish_to
                        let reply = format!("{payload} from {full_name}");
                        session
                            .publish_to_and_wait_async(
                                received_msg.context,
                                reply.as_bytes().to_vec(),
                                None,
                                None,
                            )
                            .await?;

                        println!("[{full_name}] Sent reply: {reply}");
                    }
                    Err(e) => {
                        let error_msg = e.to_string().to_lowercase();
                        if error_msg.contains("timeout") {
                            // Timeout is expected, just continue waiting
                            continue;
                        } else {
                            println!("[{full_name}] Error receiving message: {e}");
                            break;
                        }
                    }
                }
            },
            _ = signal::ctrl_c() => {
                println!("\n[{full_name}] Received Ctrl+C, shutting down gracefully...");
                println!("[{full_name}] Session state persisted — restart with --rejoin to resume.");
                break;
            }
        }
    }

    // Cleanup — do NOT delete the session so it can be restored on rejoin.
    shutdown().await?;
    println!("[{full_name}] Receiver stopped");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args = Args::parse();

    // Run the receiver
    run_receiver(args).await
}
