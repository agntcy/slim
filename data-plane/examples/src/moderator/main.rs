// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time;
use tracing::{error, info};

use prost::Message;
use slim_controller::api::proto::moderator::v1::{ModeratorMessage, moderator_message::Payload};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_mls::mls::Mls;
use slim_service::{
    FireAndForgetConfiguration, StreamingConfiguration,
    session::{Info as SessionInfo, SessionConfig, SessionDirection},
};

mod args;

#[derive(Debug)]
struct ChannelInfo {
    mls_group_id: Option<Vec<u8>>,
    session_info: Option<SessionInfo>,
    participants: std::collections::HashSet<String>,
    mls: Option<Mls<SharedSecret, SharedSecret>>,
}

impl ChannelInfo {
    fn new() -> Self {
        Self {
            mls_group_id: None,
            session_info: None,
            participants: std::collections::HashSet::new(),
            mls: None,
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

    let mut channel_info = ChannelInfo::new();

    info!("Creating MLS group for channel: {}", channel_id);

    // Create MLS instance
    let identity_provider = SharedSecret::new(moderator_name, "group");
    let identity_verifier = SharedSecret::new(moderator_name, "group");
    let moderator_name_obj = Name::from_strings(["org", "default", moderator_name]).with_id(0);
    let mls_storage_path =
        std::path::PathBuf::from(format!("/tmp/mls_moderator_{}", moderator_name));

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
    channel_info.mls = Some(mls);

    let channel_name = Name::from_strings(["channel", "channel", channel_id]).with_id(0);
    let session_info = app
        .create_session(
            SessionConfig::Streaming(StreamingConfiguration::new(
                SessionDirection::Bidirectional,
                channel_name.clone(),
                true,
                Some(10),
                Some(Duration::from_secs(1)),
                true,
            )),
            Some(channel_id.parse::<u32>().unwrap_or(12345)),
        )
        .await?;

    channel_info.session_info = Some(session_info);

    channels.insert(channel_id.to_string(), channel_info);
    info!(
        "Channel {} stored in channels map. Total channels: {}",
        channel_id,
        channels.len()
    );

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
                let session_msg = match next.unwrap() {
                    Ok(msg) => msg,
                    Err(e) => {
                        error!("Moderator: error receiving session message: {}", e);
                        continue;
                    }
                };
                info!("MODERATOR_MSG_DEBUG: session_info={:?}", session_msg.info);
                info!("MODERATOR_MSG_DEBUG: message={:?}", session_msg.message);

                // Process incoming messages
                match &session_msg.message.message_type.unwrap() {
                    slim_datapath::api::ProtoPublishType(msg) => {
                        let payload = msg.get_payload();

                        match payload.content_type.as_str() {
                            "application/x-moderator-protobuf" => {
                                match ModeratorMessage::decode(&*payload.blob) {
                                    Ok(moderator_msg) => {
                                        info!("Received moderator message: {}", moderator_msg.message_id);

                                        match moderator_msg.payload {
                                            Some(Payload::CreateChannel(req)) => {
                                                info!("Controller requested channel creation for channel_id: {}", req.channel_id);

                                                match create_channel(&app, &req.channel_id, moderator_name, &mut channels).await {
                                                    Ok(()) => {
                                                        info!("Successfully created channel: {}", req.channel_id);
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to create channel {}: {}", req.channel_id, e);
                                                    }
                                                }
                                            }
                                            Some(Payload::AddParticipant(req)) => {
                                                info!("Controller requested to add participant {} to channel {}",
                                                      req.participant_id, req.channel_id);

                                                info!("MODERATOR: Current channels in map: {:?}", channels.keys().collect::<Vec<_>>());
                                                info!("MODERATOR: Looking for channel: {}", req.channel_id);

                                                if let Some(channel_info) = channels.get_mut(&req.channel_id) {
                                                    if let Some(session_info) = &channel_info.session_info {
                                                        let participant_name = Name::from_strings(["org", "default", &req.participant_id]).with_id(0);

                                                        info!("Inviting participant {} to channel {} with session ID {}",
                                                              req.participant_id, req.channel_id, session_info.id);

                                                        // Set up route to participant before inviting
                                                        if let Err(e) = app.set_route(&participant_name, conn_id).await {
                                                            error!("Failed to set route to participant {}: {}", req.participant_id, e);
                                                        }

                                                        info!("MODERATOR: About to call invite_participant for {}", participant_name);
                                                        match app.invite_participant(&participant_name, session_info.clone()).await {
                                                            Ok(()) => {
                                                                channel_info.participants.insert(req.participant_id.clone());
                                                                info!("Successfully invited participant {} to channel {}",
                                                                      req.participant_id, req.channel_id);

                                                                // Send a trigger message to start communication after both participants are added
                                                                if channel_info.participants.len() == 2 {
                                                                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                                                                    let trigger_msg = "start communication";
                                                                    let channel_name = Name::from_strings(["channel", "channel", &req.channel_id]).with_id(0);

                                                                    info!("MODERATOR: Sending trigger message '{}' to channel {}", trigger_msg, channel_name);
                                                                    use slim_service::SlimHeaderFlags;
                                                                    let flags = SlimHeaderFlags::new(10, None, None, None, None);

                                                                    match app.publish_with_flags(session_info.clone(), &channel_name, flags, trigger_msg.into(), None, None).await {
                                                                        Ok(()) => {
                                                                            info!("MODERATOR: Successfully sent trigger message to channel");
                                                                        }
                                                                        Err(e) => {
                                                                            error!("Failed to send trigger message to channel: {}", e);
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                error!("Failed to invite participant {} to channel {}: {}",
                                                                       req.participant_id, req.channel_id, e);
                                                            }
                                                        }
                                                    } else {
                                                        error!("Channel {} has no session info", req.channel_id);
                                                    }
                                                } else {
                                                    error!("Channel {} does not exist", req.channel_id);
                                                }
                                            }
                                            Some(Payload::RemoveParticipant(req)) => {
                                                info!("Controller requested to remove participant {} from channel {}",
                                                      req.participant_id, req.channel_id);

                                                if let Some(channel_info) = channels.get_mut(&req.channel_id) {
                                                    if let Some(session_info) = &channel_info.session_info {
                                                        let participant_name = Name::from_strings(["org", "default", &req.participant_id]).with_id(0);

                                                        info!("Removing participant {} from channel {} with session ID {}",
                                                              req.participant_id, req.channel_id, session_info.id);

                                                        match app.remove_participant(&participant_name, session_info.clone()).await {
                                                            Ok(()) => {
                                                                channel_info.participants.remove(&req.participant_id);
                                                                info!("Successfully removed participant {} from channel {}",
                                                                      req.participant_id, req.channel_id);
                                                            }
                                                            Err(e) => {
                                                                error!("Failed to remove participant {} from channel {}: {}",
                                                                       req.participant_id, req.channel_id, e);
                                                            }
                                                        }
                                                    } else {
                                                        error!("Channel {} has no session info", req.channel_id);
                                                    }
                                                } else {
                                                    error!("Channel {} does not exist", req.channel_id);
                                                }
                                            }
                                            Some(Payload::DeleteChannel(req)) => {
                                                info!("Controller requested to delete channel {}", req.channel_id);

                                                if let Some(channel_info) = channels.get(&req.channel_id) {
                                                    if let Some(session_info) = &channel_info.session_info {
                                                        let participants_to_remove: Vec<String> = channel_info.participants.iter().cloned().collect();

                                                        for participant_id in participants_to_remove {
                                                            let participant_name = Name::from_strings(["org", "default", &participant_id]).with_id(0);
                                                            info!("Removing participant {} before deleting channel {}", participant_id, req.channel_id);

                                                            if let Err(e) = app.remove_participant(&participant_name, session_info.clone()).await {
                                                                error!("Failed to remove participant {} before deleting channel {}: {}", participant_id, req.channel_id, e);
                                                            }
                                                        }
                                                    }

                                                    channels.remove(&req.channel_id);
                                                    info!("Successfully deleted channel {}", req.channel_id);
                                                } else {
                                                    error!("Channel {} does not exist", req.channel_id);
                                                }
                                            }
                                            None => {
                                                error!("Received ModeratorMessage with no payload");
                                            }
                                            _ => {
                                                error!("Received unknown moderator message type");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to decode protobuf message: {}", e);
                                    }
                                }
                            }
                            _ => {
                                error!("Unsupported content type: {}", payload.content_type);
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
