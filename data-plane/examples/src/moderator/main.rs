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
                        spawn_session_receiver(session_ctx);
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
) -> std::sync::Arc<slim_service::session::Session<SharedSecret, SharedSecret>> {

    session_ctx
        .spawn_receiver(move |mut rx, _session| async move {
            info!("Received session");
            while let Some(msg) = rx.recv().await {
                match msg {
                    Ok(m) => {
                        println!("received message {:?}", m);
                    }
                    Err(e) => {
                        error!("error receiving session message: {}", e);
                    }
                }
            }

            info!("session receiver task ending");
        })
        .upgrade()
        .expect("failed to upgrade session weak reference")
}

