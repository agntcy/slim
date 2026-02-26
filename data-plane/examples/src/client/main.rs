// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use display_error_chain::ErrorChainExt;
use prost::Message;
use tracing::{error, info};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_session::Notification;

mod args;

fn spawn_session_receiver(
    session_ctx: slim_session::context::SessionContext,
    message: &Option<String>,
) {
    let message_clone = message.clone();
    session_ctx
        .spawn_receiver(|mut rx, session| async move {
            info!("Session handler task started");

            if let Some(m) = message_clone {
                if let Some(s) = session.upgrade() {
                        if let Err(e) = s.publish(s.dst(), m.encode_to_vec(), None, None).await {
                            error!(error = %e.chain(), "failed to publish message to session");
                        }
                } else {
                    error!("failed to upgrade session weak reference");
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
                                info!(?msg, "CLIENT: message details");

                                let publisher = msg.get_slim_header().get_source();
                                let msg_id = msg.get_id();
                                info!(from = %publisher, %msg_id, "CLIENT: message");

                                if let Some(c) = msg.get_payload() {
                                    let blob = &c.as_application_payload().unwrap().blob;
                                    info!(payload_length = %blob.len());

                                    match String::from_utf8(blob.to_vec()) {
                                        Ok(text) => {
                                            info!(message = %text);
                                        },
                                        Err(e) => {
                                            info!(msg_len = %blob.len(), error = %e.chain(), "received encrypted/binary message");
                                        }
                                    }
                                } else {
                                    info!("received message without payload.");
                                }
                            },
                            Some(Err(e)) => {
                                error!(error = %e.chain(), "error receiving session message");
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
    let mut loader = config::ConfigLoader::new(config_file)
        .with_context(|| format!("Failed to load configuration from {}", config_file))?;
    let _guard = loader.tracing()?.setup_tracing_subscriber();

    info!(%config_file, local=%local_name, "starting client example");

    // Obtain service (assumes service id slim/0 is present)
    let service_id = slim_config::component::id::ID::new_with_str("slim/0")
        .context("Failed to create service ID 'slim/0'")?;

    // Access mutable services map from loader
    let services = loader
        .services()
        .with_context(|| "Failed to load services from configuration")?;
    let svc = services
        .shift_remove(&service_id)
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
            SharedSecret::new(&local_name.to_string(), secret)
                .expect("Failed to create SharedSecret"),
            SharedSecret::new(&local_name.to_string(), secret)
                .expect("Failed to create SharedSecret"),
        )
        .with_context(|| format!("Failed to create app for name {}", local_name))?;

    // Start service (establish client connections)
    svc.run().await.context("Service run failed")?;

    // Connection id of first configured client
    let clients = svc.config().dataplane_clients();
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
                            error!(error = %e.chain(), "error receiving app notification");
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

    svc.shutdown().await.context("Service shutdown failed")
}
