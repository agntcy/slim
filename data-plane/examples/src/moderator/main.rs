// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::time;
use tracing::{info, error};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_mls::mls::Mls;
use slim_service::{FireAndForgetConfiguration, StreamingConfiguration, session::{SessionConfig, SessionDirection, Info as SessionInfo}};

mod args;

#[derive(Debug, Clone)]
struct ChannelInfo {
    channel_id: String,
    moderators: Vec<String>,
    mls_group_id: Option<Vec<u8>>,
    session_info: Option<SessionInfo>,
    created_at: SystemTime,
}

impl ChannelInfo {
    fn new(channel_id: String, moderators: Vec<String>) -> Self {
        Self {
            channel_id,
            moderators,
            mls_group_id: None,
            session_info: None,
            created_at: SystemTime::now(),
        }
    }
}

async fn create_channel(
    app: &slim_service::app::App<SharedSecret, SharedSecret>,
    channel_id: &str,
    moderator_name: &str,
    channels: &mut HashMap<String, ChannelInfo>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Creating channel {}", channel_id);
    
    let mut channel_info = ChannelInfo::new(
        channel_id.to_string(), 
        vec![moderator_name.to_string()]
    );
    
    info!("Creating MLS group for channel: {}", channel_id);
    
    // Create MLS instance
    let identity_provider = SharedSecret::new(moderator_name, "group");
    let identity_verifier = SharedSecret::new(moderator_name, "group");
    let moderator_name_obj = Name::from_strings(["org", "default", moderator_name]).with_id(0);
    let mls_storage_path = std::path::PathBuf::from(format!("/tmp/mls_moderator_{}", moderator_name));
    
    let mut mls = Mls::new(
        moderator_name_obj,
        identity_provider,
        identity_verifier,
        mls_storage_path,
    );
    
    mls.initialize()?;
    
    // Create MLS group
    let group_id = mls.create_group()?;
    info!("Created MLS group with ID: {:?}", group_id);
    
    channel_info.mls_group_id = Some(group_id);
    
    let channel_name = Name::from_strings(["channel", "channel", channel_id]).with_id(0);
    let session_info = app.create_session(
        SessionConfig::Streaming(StreamingConfiguration::new(
            SessionDirection::Bidirectional,
            channel_name.clone(),
            true,
            Some(10),
            Some(Duration::from_secs(1)),
            true,
        )),
        Some(channel_id.parse::<u32>().unwrap_or(12345)),
    ).await?;
    
    channel_info.session_info = Some(session_info);
    
    channels.insert(channel_id.to_string(), channel_info);
    info!("Channel {} stored in channels map. Total channels: {}", channel_id, channels.len());
    
    Ok(())
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
    let _mls_enabled = args.mls();

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
    info!("MODERATOR_CONN_ID: {:?}", conn_id);
    info!(
        "MODERATOR_ENDPOINT: {:?}",
        svc.config().clients()[0].endpoint
    );

    // Subscribe to moderator's own endpoint for control messages
    app.subscribe(&name, Some(conn_id)).await.unwrap();
    info!(
        "Moderator subscribed to: {:?} with conn_id: {:?}",
        name, conn_id
    );

    // Create a fire and forget session with ID 0 to handle incoming control messages
    let _session_info = app
        .create_session(
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
            Some(0),
        )
        .await
        .expect("failed to create session");

    // Wait for connection to be established
    tokio::time::sleep(Duration::from_millis(100)).await;

    info!("Starting moderator, waiting for requests...");
    let mut channels: HashMap<String, ChannelInfo> = HashMap::new();

    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
                break;
            }
            next = rx.recv() => {
                if next.is_none() {
                    info!("Moderator: received None message, breaking");
                    break;
                }

                info!("Moderator: received a message from rx.recv()");
                let session_msg = next.unwrap().expect("error");
                info!("MODERATOR_MSG_DEBUG: session_info={:?}", session_msg.info);
                info!("MODERATOR_MSG_DEBUG: message={:?}", session_msg.message);

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
                                    
                                    match create_channel(&app, channel_id, moderator_name, &mut channels).await {
                                        Ok(()) => {
                                            info!("Successfully created channel: {}", channel_id);
                                        }
                                        Err(e) => {
                                            error!("Failed to create channel {}: {}", channel_id, e);
                                        }
                                    }
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
