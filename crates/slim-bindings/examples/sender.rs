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
//! cargo run --example sender -- \
//!   --local agntcy/ns/bob \
//!   --shared-secret a-very-long-shared-secret-abcdef1234567890 \
//!   --session-type p2p \
//!   --participants agntcy/ns/alice
//!
//! # Group
//! cargo run --example sender -- \
//!   --local agntcy/ns/bob \
//!   --shared-secret a-very-long-shared-secret-abcdef1234567890 \
//!   --session-type group \
//!   --participants agntcy/ns/alice agntcy/ns/charlie
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use slim_bindings::{
    MlsSettings, Name, SessionConfig, SessionType, get_global_service, initialize_with_defaults,
    new_insecure_client_config, shutdown,
};

/// Command-line arguments for the sender application
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

    /// Session type: "p2p" or "group"
    #[arg(long)]
    session_type: String,

    /// List of participant identities (format: organization/namespace/application)
    /// For p2p: exactly 1 participant
    /// For group: at least 1 participant
    #[arg(long, required = true, num_args = 1..)]
    participants: Vec<String>,

    /// Number of messages to send
    #[arg(long, default_value = "10")]
    count: usize,

    /// Interval between messages in milliseconds
    #[arg(long, default_value = "100")]
    interval_ms: u64,
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

/// Parse session type from string
fn parse_session_type(s: &str) -> Result<SessionType, Box<dyn std::error::Error>> {
    match s.to_lowercase().as_str() {
        "p2p" | "point-to-point" | "pointtopoint" => Ok(SessionType::PointToPoint),
        "group" | "multicast" => Ok(SessionType::Group),
        _ => Err(format!("Invalid session type '{s}'. Expected 'p2p' or 'group'").into()),
    }
}

/// Main sender loop
async fn run_sender(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Validate arguments
    let session_type = parse_session_type(&args.session_type)?;

    if session_type == SessionType::PointToPoint && args.participants.len() != 1 {
        return Err("Point-to-point sessions require exactly 1 participant".into());
    }

    if args.participants.is_empty() {
        return Err("At least 1 participant is required".into());
    }

    // Parse the local identity
    let local_name = parse_name(&args.local)?;
    let local_name_arc = Arc::new(local_name);

    // Parse participant names
    let participant_names: Result<Vec<Name>, _> =
        args.participants.iter().map(|s| parse_name(s)).collect();
    let participant_names = participant_names?;

    // Initialize SLIM with default configuration
    initialize_with_defaults();

    // Get the global service and connect to the control plane
    let service = get_global_service();
    let client_config = new_insecure_client_config(args.slim);
    let conn_id = service.connect_async(client_config).await?;

    println!("Connected to control plane with connection ID: {conn_id}");

    // Create the slim application using global service with shared secret
    let app = service
        .create_app_with_secret_async(local_name_arc.clone(), args.shared_secret)
        .await?;

    let full_name = app.name();
    println!("[{full_name}] Application created");

    // Subscribe to local name
    app.subscribe_async(local_name_arc, Some(conn_id)).await?;

    // Create session configuration
    let session_config = SessionConfig {
        session_type: session_type.clone(),
        max_retries: Some(5),
        interval: Some(Duration::from_secs(1)),
        metadata: HashMap::new(),
        mls_settings: Some(MlsSettings::default()),
    };

    // For p2p, destination is the single participant
    // For group, destination is the group channel name
    let destination = if session_type == SessionType::Group {
        Arc::new(parse_name("agntcy/slim/test-app-channel")?)
    } else {
        Arc::new(participant_names[0].clone())
    };

    println!("[{full_name}] Creating {session_type:?} session with destination {destination}...");

    // Set routes for all participants to ensure they can receive the session invite
    for participant in &participant_names {
        println!("[{full_name}] Setting route for {participant}");
        app.set_route_async(Arc::new(participant.clone()), conn_id)
            .await?;
    }

    let session_with_completion = app
        .create_session_async(session_config, destination)
        .await?;

    // Wait for session establishment
    session_with_completion.completion.wait_async().await?;
    let session = session_with_completion.session;

    let session_id = session.session_id()?;
    println!("[{full_name}] Session {session_id} established");

    // For group sessions, invite all participants
    if session_type == SessionType::Group {
        for participant in &participant_names {
            println!("[{full_name}] Inviting {participant} to session...");
            session
                .invite_and_wait_async(Arc::new(participant.clone()))
                .await?;
            println!("[{full_name}] {participant} joined session");
        }
    }

    let interval = Duration::from_millis(args.interval_ms);
    let expected_replies_per_message = participant_names.len();
    let total_expected_replies = args.count * expected_replies_per_message;

    println!(
        "[{}] Sending {} messages with {}ms interval...",
        full_name, args.count, args.interval_ms
    );

    // Spawn a background task to collect replies
    let session_clone = session.clone();
    let full_name_clone = full_name.clone();
    let reply_task = tokio::spawn(async move {
        let mut total_replies_received = 0;

        loop {
            match session_clone
                .get_message_async(Some(Duration::from_secs(10)))
                .await
            {
                Ok(received_msg) => {
                    let payload = String::from_utf8_lossy(&received_msg.payload);
                    let source = received_msg.context.source_name.to_string();

                    println!("[{full_name_clone}] Reply from {source}: {payload}");

                    total_replies_received += 1;

                    // Stop if we've received all expected replies
                    if total_replies_received >= total_expected_replies {
                        break;
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string().to_lowercase();
                    if error_msg.contains("timeout") {
                        // Continue waiting
                        continue;
                    } else {
                        println!("[{full_name_clone}] Error receiving reply: {e}");
                        break;
                    }
                }
            }
        }

        total_replies_received
    });

    // Send messages at fixed intervals
    for i in 0..args.count {
        let message = format!("Message {}", i + 1);
        println!("[{full_name}] Sending: {message}");

        if let Err(e) = session
            .publish_and_wait_async(message.as_bytes().to_vec(), None, None)
            .await
        {
            println!("[{full_name}] Error sending message: {e}");
        }

        tokio::time::sleep(interval).await;
    }

    println!("[{full_name}] Finished sending messages, waiting for remaining replies...");

    // Wait for reply collection task to finish or timeout
    let total_replies_received = tokio::select! {
        result = reply_task => {
            match result {
                Ok(data) => data,
                Err(e) => {
                    println!("[{full_name}] Reply collection task error: {e}");
                    0
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(30)) => {
            println!("[{full_name}] Timeout waiting for replies");
            0
        }
    };

    // Summary
    println!("\n[{full_name}] === Summary ===");
    println!("[{full_name}] Replies received: {total_replies_received}/{total_expected_replies}");

    if total_replies_received == total_expected_replies {
        println!("[{full_name}] ✓ All participants replied correctly");
    } else {
        println!(
            "[{}] ✗ Missing {} replies",
            full_name,
            total_expected_replies - total_replies_received
        );
    }

    // Delete session
    println!("[{full_name}] Deleting session...");
    app.delete_session_and_wait_async(session).await?;

    // Cleanup
    shutdown().await?;
    println!("[{full_name}] Sender stopped");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args = Args::parse();

    // Run the sender
    run_sender(args).await
}
