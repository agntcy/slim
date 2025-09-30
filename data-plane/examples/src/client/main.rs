// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tokio::time;
use tracing::{error, info};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::session::Notification;

mod args;

fn spawn_session_receiver(
    session_ctx: slim_service::session::context::SessionContext<SharedSecret, SharedSecret>,
    local_name: Name,
) -> std::sync::Arc<slim_service::session::Session<SharedSecret, SharedSecret>> {
    session_ctx
        .spawn_receiver(|mut rx, session| async move {
            info!("Session handler task started");

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
                                let dst = msg.get_slim_header().get_dst();
                                let msg_id = msg.get_id();
                                info!("CLIENT: message from {:?}, id: {}", publisher, msg_id);

                                if let Some(c) = msg.get_payload() {
                                    let blob = &c.blob;
                                    info!("CLIENT: message has payload of {} bytes", blob.len());

                                    match String::from_utf8(blob.to_vec()) {
                                        Ok(text) => {
                                            info!("received message: {}", text);
                                            let response = format!("hello from the {}", local_name);
                                            info!("CLIENT: sending response: {}", response);
                                            let _ = session.upgrade().expect("failed to get session reference").publish(&dst, response.into(), None, None).await;
                                        },
                                        Err(e) => {
                                            info!("received encrypted/binary message: {} bytes, error: {}", blob.len(), e);
                                        }
                                    }
                                } else {
                                    info!("received message without payload (possibly invitation)");
                                }
                            },
                            Some(Err(e)) => {
                                error!("error receiving session message: {}", e);
                                continue;
                            }
                        };
                    }
                }
            }
        })
        .upgrade()
        .expect("failed to upgrade session Arc")
}

#[tokio::main]
async fn main() {
    // Parse CLI
    let args = args::Args::parse();

    // Take owned copies of CLI inputs so we don't hold references to Args across awaits/spawn
    let config_file = args.config().to_string();
    let local_name = args.local_name();

    // Load configuration
    let mut config = config::load_config(&config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, local=%local_name, "starting client example");

    // Obtain service (assumes service id slim/0 is present)
    let service_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config
        .services
        .remove(&service_id)
        .expect("missing service slim/0 in configuration");

    // Build Names
    let local_name = Name::from_strings(["org", "default", local_name]).with_id(0);

    // Create app
    let (app, mut app_rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&local_name.to_string(), "group"),
            SharedSecret::new(&local_name.to_string(), "group"),
        )
        .await
        .expect("failed to create app");

    // Start service (establish client connections)
    svc.run().await.expect("service run failed");

    // Connection id of first configured client
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .expect("missing connection id");
    info!(%conn_id, "connection established");

    // Subscribe to our own name for potential direct control or discovery messages
    app.subscribe(&local_name, Some(conn_id))
        .await
        .expect("failed to subscribe local name");

    // Allow a brief delay for subscription/route propagation
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    info!("CLIENT: Entering message receive loop");

    let mut sessions = Vec::new();
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
                            error!("error receiving app notification: {}", e);
                            continue;
                        }
                    }
                };

                match notification {
                    Notification::NewSession(ctx) => {
                        // New remotely-initiated session. Spawn a task to handle it.
                        sessions.push(spawn_session_receiver(ctx, local_name.clone()));
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

    // delete all sessions
    for session in sessions.into_iter() {
        let session_id = session.id();
        info!(%session_id, "deleting session");
        if let Err(e) = app.delete_session(&session).await {
            error!(%session_id, "failed to delete session: {}", e);
        }
    }

    let signal = svc.signal();
    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => info!("service drained"),
        Err(_) => error!("timeout waiting for service drain"),
    }
}
