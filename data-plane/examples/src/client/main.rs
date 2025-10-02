// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use prost::Message;
use tokio::time;
use tracing::{error, info};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::session::Notification;

mod args;

fn spawn_session_receiver(
    session_ctx: slim_service::session::context::SessionContext<SharedSecret, SharedSecret>,
    message: &Option<String>,
) {
    let message_clone = message.clone();
    session_ctx
        .spawn_receiver(|mut rx, session| async move {
            info!("Session handler task started");

            if let Some(m) = message_clone {
                if let Some(s) = session.upgrade() {
                    if let Some(dst) = s.dst() {
                        if let Err(e) = s.publish(&dst, m.encode_to_vec(), None, None).await {
                            error!("Failed to publish message to session: {:?}", e);
                        }
                    } else {
                        error!("Failed to get session destination");
                    }
                } else {
                    error!("Failed to upgrade session weak reference");
                }
            }

            loop {
                tokio::select! {
                    message = rx.recv() => {
                        match message {
                            None => {
                                info!("session closed");
                                break;
                            }
                            Some(Ok(msg)) => {
                                info!("CLIENT: received something from rx.recv()");
                                info!("CLIENT: message details: {:?}", msg);

                                let publisher = msg.get_slim_header().get_source();
                                let msg_id = msg.get_id();
                                info!("CLIENT: message from {:?}, id: {}", publisher, msg_id);

                                if let Some(c) = msg.get_payload() {
                                    let blob = &c.blob;
                                    info!("CLIENT: message has payload of {} bytes", blob.len());

                                    match String::from_utf8(blob.to_vec()) {
                                        Ok(text) => {
                                            info!("received message: {}", text);
                                        },
                                        Err(e) => {
                                            info!("received encrypted/binary message: {} bytes, error: {}", blob.len(), e);
                                        }
                                    }
                                } else {
                                    info!("received message without payload.");
                                }
                            },
                            Some(Err(e)) => {
                                error!("error receiving session message: {:?}", e);
                                continue;
                            }
                        };
                    }
                }
            }
        });
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI
    let args = args::Args::parse();

    let config_file = args.config();
    let local_name = args.local_name();
    let message = args.message();
    let secret = args.secret();

    // Load configuration
    let mut config = config::load_config(config_file)
        .with_context(|| format!("Failed to load configuration from {}", config_file))?;
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, local=%local_name, "starting client example");

    // Obtain service (assumes service id slim/0 is present)
    let service_id = slim_config::component::id::ID::new_with_str("slim/0")
        .context("Failed to create service ID 'slim/0'")?;
    let mut svc = config
        .services
        .remove(&service_id)
        .context("Missing service 'slim/0' in configuration")?;

    // Build Names
    let comp = local_name.split("/").collect::<Vec<&str>>();
    if comp.len() < 3 {
        anyhow::bail!(
            "Local name '{}' must have at least 3 components separated by '/'",
            local_name
        );
    }
    let local_name = Name::from_strings([comp[0], comp[1], comp[2]]).with_id(0);

    // Create app
    let (app, mut app_rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&local_name.to_string(), secret),
            SharedSecret::new(&local_name.to_string(), secret),
        )
        .await
        .with_context(|| format!("Failed to create app for name {}", local_name))?;

    // Start service (establish client connections)
    svc.run().await.context("Service run failed")?;

    // Connection id of first configured client
    let clients = svc.config().clients();
    if clients.is_empty() {
        anyhow::bail!("No clients configured in service");
    }

    let conn_id = svc
        .get_connection_id(&clients[0].endpoint)
        .with_context(|| format!("Missing connection id for endpoint {}", clients[0].endpoint))?;
    info!(%conn_id, "connection established");

    // Subscribe to our own name for potential direct control or discovery messages
    app.subscribe(&local_name, Some(conn_id))
        .await
        .with_context(|| format!("Failed to subscribe to local name {}", local_name))?;

    // Allow a brief delay for subscription/route propagation
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    info!("CLIENT: Entering message receive loop");

    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("shutdown signal received");
                break;
            }
            notification = app_rx.recv() => {
                let notification = match notification {
                    None => {
                        info!("app notification channel closed");
                        break;
                    }
                    Some(res) => match res {
                        Ok(n) => n,
                        Err(e) => {
                            error!("error receiving app notification: {:?}", e);
                            continue;
                        }
                    }
                };

                match notification {
                    Notification::NewSession(ctx) => {
                        // New remotely-initiated session. Spawn a task to handle it.
                        spawn_session_receiver(ctx, message);
                    }
                    Notification::NewMessage(_msg) => {
                        // Application-level publish without an associated session.
                        // This example does not expect such messages, so we just log.
                        error!("received unexpected app-level NewMessage; ignoring");
                    }
                }
            }
        }
    }

    info!("client shutting down");

    let signal = svc.signal();
    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => info!("service drained"),
        Err(_) => error!(
            "timeout waiting for service drain after {:?}",
            config.runtime.drain_timeout()
        ),
    }

    Ok(())
}
