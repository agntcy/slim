// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time;
use tracing::{error, info};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::{Name, utils::SlimHeaderFlags};
use slim_service::{
    session::{self, SessionConfig, SessionDirection},
    streaming::StreamingConfiguration,
};

mod args;

struct ChannelState {
    session_info: session::Info,
    channel_name: Name,
    participant_count: u32,
}

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let args = args::Args::parse();

    // Get config file
    let config_file = args.config();

    // Get moderator name
    let moderator_name = args.name();

    // Get MLS setting for created channels
    let mls_enabled = args.mls();

    // Create configured components
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, %moderator_name, "starting moderator");

    // Get service
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // Create moderator app
    let id = 0;
    let name = Name::from_strings(["org", "default", moderator_name]).with_id(id);
    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(moderator_name, "group"),
            SharedSecret::new(moderator_name, "group"),
        )
        .await
        .expect("failed to create app");

    // Run the service
    svc.run().await.unwrap();

    // Get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    // Subscribe to moderator's own endpoint for control messages
    app.subscribe(&name, Some(conn_id)).await.unwrap();
    info!("Moderator subscribed to: {:?}", name);

    // Wait for connection to be established
    tokio::time::sleep(Duration::from_millis(100)).await;

    info!("Starting moderator, waiting for requests...");

    // Store channels
    let mut channels: HashMap<String, ChannelState> = HashMap::new();

    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
                break;
            }
            next = rx.recv() => {
                if next.is_none() {
                    break;
                }

                let session_msg = next.unwrap().expect("error");

                // Process incoming messages
                match &session_msg.message.message_type.unwrap() {
                    slim_datapath::api::ProtoPublishType(msg) => {
                        let payload = msg.get_payload();

                        match std::str::from_utf8(&payload.blob) {
                            Ok(text) => {
                                info!("Received request: {}", text);
                                
                                // Check if this is a channel creation request from controller
                                if text.starts_with("create_channel:") {
                                    let channel_id = text.strip_prefix("create_channel:").unwrap_or("");
                                    info!("Controller requested channel creation for channel_id: {}", channel_id);
                                }
                            }
                            Err(_) => {
                                info!("Received unknown message type");
                            }
                        }
                    }
                    t => {
                        info!("Received message type: {:?}", t);
                    }
                }
            }
        }
    }

    info!("Moderator shutting down");

    let signal = svc.signal();

    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => {}
        Err(_) => panic!("Timeout waiting for drain for service"),
    }
}
