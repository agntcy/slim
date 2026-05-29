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
//! cargo run --example receiver -- \
//!   --local agntcy/ns/alice \
//!   --shared-secret a-very-long-shared-secret-abcdef1234567890
//! ```

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use slim_bindings::{
    Name, get_global_service, initialize_with_defaults, new_insecure_client_config, shutdown,
};
use tokio::signal;

/// Command-line arguments for the receiver application
#[derive(Parser, Debug)]
struct Args {
    /// Local identity in format: organization/namespace/application
    #[arg(long)]
    local: String,

    /// Shared secret for authentication
    #[arg(long)]
    shared_secret: String,

    /// SLIM control plane endpoint
    #[arg(long, default_value = "http://localhost:46357")]
    slim: String,
}

/// Parse a name string in format "org/namespace/app" into a Name object
fn parse_name(id: &str) -> Result<Name, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() != 3 {
        return Err(format!(
            "Invalid name format '{}'. Expected format: organization/namespace/application",
            id
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
    // Parse the local identity
    let local_name = parse_name(&args.local)?;
    let local_name_arc = Arc::new(local_name);

    // Initialize SLIM with default configuration
    initialize_with_defaults();

    // Get the global service and connect to the control plane
    let service = get_global_service();
    let client_config = new_insecure_client_config(args.slim);
    let conn_id = service.connect_async(client_config).await?;

    println!("Connected to control plane with connection ID: {}", conn_id);

    // Create the slim application using global service with shared secret
    let app = service
        .create_app_with_secret_async(local_name_arc.clone(), args.shared_secret)
        .await?;

    let full_name = app.name();

    // Subscribe to local name
    app.subscribe_async(local_name_arc, Some(conn_id)).await?;

    println!("[{}] Waiting for incoming session...", full_name);

    // Wait for one incoming session (no timeout)
    let session = app.listen_for_session_async(None).await?;

    let session_id = session.session_id()?;
    let destination = session.destination()?;
    println!(
        "[{}] New session {} established from {}",
        full_name, session_id, destination
    );

    // Loop to receive messages and reply
    loop {
        tokio::select! {
            result = session.get_message_async(Some(Duration::from_secs(5))) => {
                match result {
                    Ok(received_msg) => {
                        let payload = String::from_utf8_lossy(&received_msg.payload);
                        let source = &received_msg.context.source_name;

                        println!(
                            "[{}] Received from {}: {}",
                            full_name, source, payload
                        );

                        // Reply to the sender using publish_to
                        let reply = format!("{} from {}", payload, full_name);
                        session
                            .publish_to_and_wait_async(
                                received_msg.context,
                                reply.as_bytes().to_vec(),
                                None,
                                None,
                            )
                            .await?;

                        println!("[{}] Sent reply: {}", full_name, reply);
                    }
                    Err(e) => {
                        let error_msg = e.to_string().to_lowercase();
                        if error_msg.contains("timeout") {
                            // Timeout is expected, just continue waiting
                            continue;
                        } else {
                            println!("[{}] Error receiving message: {}", full_name, e);
                            break;
                        }
                    }
                }
            },
            _ = signal::ctrl_c() => {
                println!("\n[{}] Received Ctrl+C, shutting down gracefully...", full_name);
                break;
            }
        }
    }

    // Cleanup
    shutdown().await?;
    println!("[{}] Receiver stopped", full_name);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args = Args::parse();

    // Run the receiver
    run_receiver(args).await
}
