// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use clap::Parser;
use display_error_chain::ErrorChainExt;
use tracing::info;

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::{api::ProtoSessionType, messages::Name};
use slim_session::SessionConfig;
use slim_testing::utils::TEST_VALID_SECRET;

mod args;

// Function to handle session messages using spawn_receiver
fn spawn_session_receiver(
    session_ctx: slim_session::context::SessionContext,
    local_name: String,
    route: Name,
) {
    session_ctx.spawn_receiver(|mut rx, weak| async move {
            info!("Session handler task started");

            // Local deque for queuing reply messages
            let mut reply_queue = std::collections::VecDeque::<String>::new();

            // Timer for periodic message sending (every second)
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

            loop {
                tokio::select! {
                    // Handle incoming messages
                    msg_result = rx.recv() => {
                        match msg_result {
                            Some(Ok(message)) => match &message.message_type {
                                Some(slim_datapath::api::MessageType::Publish(msg)) => {
                                    let payload = msg.get_payload();
                                    match std::str::from_utf8(&payload.as_application_payload().unwrap().blob) {
                                        Ok(text) => {
                                            info!(msg = %text, "received message");
                                        }
                                        Err(_) => {
                                            info!(
                                                msg_len = %payload.as_application_payload().unwrap().blob.len(),
                                                "received encrypted message",
                                            );
                                        }
                                    }

                                    // Queue reply message instead of sending immediately
                                    let reply = format!("hello from the {}", local_name);
                                    info!(reply = %reply, "Queueing reply message");
                                    reply_queue.push_back(reply);
                                }
                                _ => {
                                    info!("received non-publish message");
                                }
                            },
                            Some(Err(e)) => {
                                info!(error = %e.chain(), "received message error");
                                break;
                            }
                            None => {
                                info!("Message channel closed");
                                break;
                            }
                        }
                    }

                    // Periodic timer - send queued messages
                    _ = interval.tick() => {
                        if let Some(reply) = reply_queue.pop_front() {
                            info!(%reply, "Sending periodic reply");

                            if let Some(session_arc) = weak.upgrade() {
                                let reply_bytes = reply.into_bytes();
                                if let Err(e) = session_arc
                                    .publish(&route, reply_bytes, None, None).await
                                {
                                    info!(error = %e.chain(), "error sending periodic reply");
                                }
                            } else {
                                info!("session already dropped; cannot send reply");
                                break;
                            }
                        }
                    }
                }
            }
            info!("Session handler task ended");
        });
}

#[tokio::main]
async fn main() {
    // parse command line arguments
    let args = args::Args::parse();

    // get config file
    let config_file = args.config();

    // get local name
    let local_name = args.local_name();

    // get remote name
    let remote_name = args.remote_name();

    // get message
    let message = args.message();

    // get MLS group identifier
    let mls_group_id = args.mls_group_id();

    // create configured components using ConfigLoader
    let mut loader = config::ConfigLoader::new(config_file).expect("failed to load configuration");
    let _guard = loader
        .tracing()
        .expect("invalid tracing configuration")
        .setup_tracing_subscriber();

    info!(%config_file, %local_name, %remote_name, "starting client");

    // get service
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let services = loader.services().expect("failed to load services");
    let svc = services.shift_remove(&id).unwrap();

    // create local app
    let id = 0;
    let name = Name::from_strings(["org", "default", local_name]).with_id(id);
    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(local_name, TEST_VALID_SECRET)
                .expect("Failed to create SharedSecret"),
            SharedSecret::new(local_name, TEST_VALID_SECRET)
                .expect("Failed to create SharedSecret"),
        )
        .expect("failed to create app");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .unwrap();

    let local_app_name = Name::from_strings(["org", "default", local_name]).with_id(id);
    app.subscribe(&local_app_name, Some(conn_id)).await.unwrap();

    // Set a route for the remote app
    let remote_app_name = Name::from_strings(["org", "default", remote_name]);
    info!(remote_app = %remote_app_name, "allowing messages to remote app");
    app.set_route(&remote_app_name, conn_id).await.unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Local array of created sessions
    let mut sessions = vec![];

    // check what to do with the message
    if let Some(msg) = message {
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: None,
            interval: None,
            mls_enabled: mls_group_id.is_some(),
            initiator: true,
            metadata: HashMap::new(),
        };
        let session_ctx = app
            .create_session(config, remote_app_name.clone(), None)
            .await
            .expect("error creating p2p session");

        // Get the session and spawn receiver for handling responses
        let (session_ctx, init_ack) = session_ctx;

        // Get session before spawning receiver
        let session = session_ctx.session_arc().unwrap();

        // Spawn receiver to handle incoming messages
        spawn_session_receiver(session_ctx, local_name.to_string(), remote_app_name.clone());

        // Await session initialization
        init_ack.await.expect("error initializing p2p session");

        info!(destination = %remote_app_name, "Sending message");

        // publish message using session context
        session
            .publish(&remote_app_name, msg.into(), None, None)
            .await
            .unwrap();

        sessions.push(session);
    }

    // wait for messages and handle sessions
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

                let notification = next.unwrap().unwrap();
                match notification {
                    slim_session::notification::Notification::NewSession(session) => {
                        info!("New session created");

                        // Get session before spawning receiver
                        let session_arc = session.session_arc().unwrap();

                        // Use the extracted spawn_session_receiver function
                        spawn_session_receiver(
                            session,
                            local_name.to_string(),
                            remote_app_name.clone(),
                        );

                        // Save the session
                        sessions.push(session_arc);
                    }
                    _ => {
                        info!("Unexpected notification type");
                    }
                }
            }
        }
    }

    info!("sdk-mock shutting down");

    // Delete all the sessions
    for session in sessions {
        app.delete_session(&session).unwrap();
    }

    // Gracefully shutdown the app
    svc.shutdown().await.unwrap();
}
