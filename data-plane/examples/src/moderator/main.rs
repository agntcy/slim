// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use prost::Message;
use slim_service::app::App;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{error, info, warn};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::{
    MulticastConfiguration, SlimHeaderFlags,
    session::{Notification, SessionConfig},
};

use slim_controller::api::proto::moderator::v1::{ModeratorMessage, moderator_message::Payload};

mod args;

// (Debug derive removed; SessionContext generic includes a transmitter type without Debug)
struct ChannelInfo {
    channel_name: Name,
    session: Arc<slim_service::session::Session<SharedSecret, SharedSecret>>,
    participants: HashSet<String>,
}

impl ChannelInfo {
    fn new(
        channel_name: Name,
        session: Arc<slim_service::session::Session<SharedSecret, SharedSecret>>,
    ) -> Self {
        Self {
            channel_name,
            session,
            participants: HashSet::new(),
        }
    }
}

async fn create_channel(
    app: &slim_service::app::App<SharedSecret, SharedSecret>,
    channel_id: &str,
    mls_enabled: bool,
) -> Result<ChannelInfo, Box<dyn std::error::Error + Send + Sync>> {
    info!("Creating channel '{}'", channel_id);

    let channel_name = Name::from_strings(["channel", "channel", channel_id]);

    let session_ctx = app
        .create_session(
            SessionConfig::Multicast(MulticastConfiguration::new(
                channel_name.clone(),
                Some(10),
                Some(Duration::from_secs(1)),
                mls_enabled,
                HashMap::new(),
            )),
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "failed to create multicast session for channel {}: {}",
                channel_id, e
            )
        })?;
    let session_arc = session_ctx
        .session_arc()
        .expect("session just created must be available");

    Ok(ChannelInfo::new(channel_name, session_arc))
}

#[tokio::main]
async fn main() {
    // Parse args
    let args = args::Args::parse();
    let config_file = args.config();
    let moderator_name = args.name();
    let mls_enabled = args.mls();

    // Load configuration
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, %moderator_name, %mls_enabled, "starting moderator example");

    // Acquire the service (assumes service id "slim/0" exists in config)
    let service_id =
        slim_config::component::id::ID::new_with_str("slim/0").expect("invalid service id");
    let mut svc = config
        .services
        .remove(&service_id)
        .expect("missing service slim/0 in configuration");

    // Build local moderator Name
    let local_name = Name::from_strings(["org", "default", moderator_name]).with_id(0);

    // Create app
    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(moderator_name, "group"),
            SharedSecret::new(moderator_name, "group"),
        )
        .await
        .expect("failed to create moderator app");

    // Run service (establish connections)
    svc.run().await.expect("service run failed");

    // Resolve connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .expect("no connection id found");
    info!(%conn_id, "remote connection established");

    // Subscribe to own endpoint to receive control messages
    app.subscribe(&local_name, Some(conn_id))
        .await
        .expect("failed to subscribe moderator endpoint");

    // Small delay to ensure subscription is sent
    tokio::time::sleep(Duration::from_millis(100)).await;

    info!("Moderator ready, waiting for control messages...");

    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("shutdown signal received");
                break;
            }
            maybe_notification = rx.recv() => {
                let notification = match maybe_notification {
                    None => {
                        info!("app channel closed");
                        break;
                    }
                    Some(res) => match res {
                        Ok(n) => n,
                        Err(e) => {
                            error!("error receiving notification: {}", e);
                            continue;
                        }
                    }
                };

                match notification {
                    Notification::NewSession(session_ctx) => {
                        info!("new session established");

                        // Spawn a task to handle session notifications
                        spawn_session_receiver(session_ctx, app.clone(), mls_enabled, conn_id);
                    }
                    Notification::NewMessage(_msg) => {
                        error!("received unexpected direct message; ignoring");
                    }
                }
            }
        }
    }

    info!("Moderator shutting down");

    let signal = svc.signal();
    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => info!("service drained"),
        Err(_) => error!("timeout draining service"),
    }
}

fn spawn_session_receiver(
    session_ctx: slim_service::session::context::SessionContext<SharedSecret, SharedSecret>,
    app: App<SharedSecret, SharedSecret>,
    mls_enabled: bool,
    conn_id: u64,
) -> std::sync::Arc<slim_service::session::Session<SharedSecret, SharedSecret>> {
    let app_clone = app.clone();

    session_ctx
        .spawn_receiver(move |mut rx, _session| async move {
            info!("Received session");

            let mut channels = HashMap::new();

            while let Some(msg) = rx.recv().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        error!("error receiving session message: {}", e);
                        continue;
                    }
                };

                // Only process publish messages with the expected content type
                if let Some(slim_datapath::api::ProtoPublishType(publish)) =
                    msg.message_type.as_ref()
                {
                    let payload = publish.get_payload();
                    if payload.content_type == "application/x-moderator-protobuf" {
                        match ModeratorMessage::decode(&*payload.blob) {
                            Ok(m) => {
                                handle_moderator_message(
                                    &m,
                                    mls_enabled,
                                    &app_clone,
                                    conn_id,
                                    &mut channels,
                                )
                                .await;
                            }
                            Err(e) => {
                                error!("failed to decode ModeratorMessage protobuf: {}", e);
                            }
                        }
                    } else {
                        // Ignore other content types
                        error!(
                            "ignoring message with unexpected content type: {}",
                            payload.content_type
                        );
                        continue;
                    }
                }
            }

            info!("session receiver task ending");
        })
        .upgrade()
        .expect("failed to upgrade session weak reference")
}

async fn handle_moderator_message(
    msg: &ModeratorMessage,
    mls_enabled: bool,
    app: &slim_service::app::App<SharedSecret, SharedSecret>,
    conn_id: u64,
    channels: &mut HashMap<String, ChannelInfo>,
) {
    match &msg.payload {
        Some(Payload::CreateChannel(req)) => {
            let channel_id = req.channel_id.clone();

            info!(
                "Controller requested channel creation for channel_id: {}",
                channel_id
            );
            if channels.contains_key(&channel_id) {
                warn!(
                    "channel '{}' already exists, ignoring CreateChannel",
                    channel_id
                );
                return;
            }
            match create_channel(app, &channel_id, mls_enabled).await {
                Ok(info_struct) => {
                    info!("created channel '{}'", channel_id);
                    channels.insert(channel_id, info_struct);
                }
                Err(e) => error!("failed to create channel '{}': {}", channel_id, e),
            }
        }
        Some(Payload::AddParticipant(req)) => {
            let channel_id = &req.channel_id;
            let participant_id = &req.participant_id;
            let Some(channel) = channels.get_mut(channel_id) else {
                error!("AddParticipant: channel '{}' not found", channel_id);
                return;
            };

            let participant_name =
                Name::from_strings(["org", "default", participant_id]).with_id(0);

            // Ensure route exists before inviting
            if let Err(e) = app.set_route(&participant_name, conn_id).await {
                error!(
                    "failed to set route for participant '{}': {}",
                    participant_id, e
                );
                return;
            }

            let session_arc = channel.session.clone();
            match session_arc.invite_participant(&participant_name).await {
                Ok(_) => {
                    channel.participants.insert(participant_id.clone());
                    info!(
                        "invited participant '{}' to channel '{}'",
                        participant_id, channel_id
                    );

                    // When we have at least two participants, send a trigger message.
                    if channel.participants.len() == 2 {
                        let flags = SlimHeaderFlags::new(10, None, None, None, None);
                        let trigger = b"start communication".to_vec();
                        if let Err(e) = session_arc
                            .publish_with_flags(&channel.channel_name, flags, trigger, None, None)
                            .await
                        {
                            error!(
                                "failed to send trigger message for channel '{}': {}",
                                channel_id, e
                            );
                        } else {
                            info!("sent trigger message for channel '{}'", channel_id);
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "failed to invite participant '{}' to channel '{}': {}",
                        participant_id, channel_id, e
                    );
                }
            }
        }
        Some(Payload::RemoveParticipant(req)) => {
            let channel_id = &req.channel_id;
            let participant_id = &req.participant_id;
            let Some(channel) = channels.get_mut(channel_id) else {
                error!("RemoveParticipant: channel '{}' not found", channel_id);
                return;
            };
            let participant_name =
                Name::from_strings(["org", "default", participant_id]).with_id(0);

            let session_arc = channel.session.clone();
            match session_arc.remove_participant(&participant_name).await {
                Ok(_) => {
                    channel.participants.remove(participant_id);
                    info!(
                        "removed participant '{}' from channel '{}'",
                        participant_id, channel_id
                    );
                }
                Err(e) => {
                    error!(
                        "failed to remove participant '{}' from channel '{}': {}",
                        participant_id, channel_id, e
                    );
                }
            }
        }
        Some(Payload::DeleteChannel(req)) => {
            let channel_id = &req.channel_id;
            let Some(channel) = channels.remove(channel_id) else {
                error!("DeleteChannel: channel '{}' not found", channel_id);
                return;
            };

            let session_arc = &channel.session;
            for p in &channel.participants {
                let name = Name::from_strings(["org", "default", p]).with_id(0);
                let _ = session_arc.remove_participant(&name).await;
            }

            info!("deleted channel '{}'", channel_id);
        }
        None => {
            error!("ModeratorMessage received with empty payload");
        }
        // Other response variants are ignored in this simple example
        _ => {
            info!("received a ModeratorMessage response variant; ignoring in example");
        }
    }
}
