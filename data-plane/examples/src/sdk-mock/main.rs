// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tokio::time;
use tracing::info;

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_auth::testutils::TEST_VALID_SECRET;
use slim_datapath::{api::ProtoSessionType, messages::Name};
use slim_session::session_controller::{SessionConfig, SessionController};

mod args;

// Function to handle session messages using spawn_receiver
fn spawn_session_receiver(
    session_ctx: slim_session::context::SessionContext<SharedSecret, SharedSecret>,
    local_name: String,
    route: Name,
) -> std::sync::Arc<SessionController<SharedSecret, SharedSecret>> {
    session_ctx
        .spawn_receiver(|mut rx, weak| async move {
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
                                    match std::str::from_utf8(&payload.as_application_payload().blob) {
                                        Ok(text) => {
                                            info!("received message: {}", text);
                                        }
                                        Err(_) => {
                                            info!(
                                                "received encrypted message: {} bytes",
                                                payload.as_application_payload().blob.len()
                                            );
                                        }
                                    }

                                    // Queue reply message instead of sending immediately
                                    let reply = format!("hello from the {}", local_name);
                                    info!("Queueing reply message: {}", reply);
                                    reply_queue.push_back(reply);
                                }
                                _ => {
                                    info!("received non-publish message");
                                }
                            },
                            Some(Err(e)) => {
                                info!("received message error: {:?}", e);
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
                            info!("Sending periodic reply: {}", reply);

                            if let Some(session_arc) = weak.upgrade() {
                                let reply_bytes = reply.into_bytes();
                                if let Err(e) = session_arc
                                    .publish(&route, reply_bytes, None, None)
                                    .await
                                {
                                    info!("error sending periodic reply: {}", e);
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
        })
        .upgrade()
        .unwrap()
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

    // create configured components
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, %local_name, %remote_name, "starting client");

    // get service
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // create local app
    let id = 0;
    let name = Name::from_strings(["org", "default", local_name]).with_id(id);
    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
        )
        .await
        .expect("failed to create app");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    let local_app_name = Name::from_strings(["org", "default", local_name]).with_id(id);
    app.subscribe(&local_app_name, Some(conn_id)).await.unwrap();

    // Set a route for the remote app
    let remote_app_name = Name::from_strings(["org", "default", remote_name]);
    info!("allowing messages to remote app: {:?}", remote_app_name);
    app.set_route(&remote_app_name, conn_id).await.unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // MLS setup, only if mls_group_id is provided
    if let Some(group_identifier) = mls_group_id {
        info!("MLS enabled with group identifier: {}", group_identifier);

        //TODO(zkacsand): temporary file based key package exchange, until the session API is ready to support it
        // Clean up previous run files
        let key_package_path = format!("/tmp/mls_key_package_{}", group_identifier);
        let welcome_path = format!("/tmp/mls_welcome_{}", group_identifier);
        let _ = std::fs::remove_file(&key_package_path);
        let _ = std::fs::remove_file(&welcome_path);

        // Clean up MLS identity directories
        let identity_path = format!("/tmp/mls_identities_{}", local_name);
        let _ = std::fs::remove_dir_all(&identity_path);

        if message.is_some() {
            // Client: will join group after server creates it
            None
        } else {
            // Server: create group and wait for client key package
            let identity_provider = SharedSecret::new("server", TEST_VALID_SECRET);
            let identity_verifier = SharedSecret::new("server", TEST_VALID_SECRET);
            let mut server_mls = slim_mls::mls::Mls::new(
                name.clone(),
                identity_provider,
                identity_verifier,
                std::path::PathBuf::from("/tmp/server_mls"),
            );
            server_mls.initialize().unwrap();

            // Create group
            let group_id = server_mls.create_group().unwrap();
            info!("Server created MLS group");

            // Wait for client key package
            info!(
                "Server waiting for client key package at: {}",
                key_package_path
            );
            let mut attempts = 0;
            let key_package = loop {
                if std::path::Path::new(&key_package_path).exists() {
                    let key_package_bytes = std::fs::read(&key_package_path).unwrap();
                    info!("Server found client key package");
                    break key_package_bytes;
                }
                if attempts > 100 {
                    panic!("Timeout waiting for client key package");
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                attempts += 1;
            };

            // Add client to group and generate welcome message
            let ret = server_mls.add_member(&key_package).unwrap();

            // Save welcome message for client
            std::fs::write(&welcome_path, &ret.welcome_message).unwrap();
            info!("Server saved welcome message to: {}", welcome_path);

            Some((server_mls, group_id))
        }
    } else {
        info!("MLS disabled - no group identifier provided");
        None
    };

    // Local array of created sessions
    let mut sessions = vec![];

    // check what to do with the message
    if let Some(msg) = message {
        let config = SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
        let session_ctx = app
            .create_session(config, remote_app_name.clone(), None)
            .await
            .expect("error creating p2p session");

        // Client MLS setup, only if mls_group_id is provided
        if let Some(group_identifier) = mls_group_id {
            // Client: generate key package and wait for welcome message
            let identity_provider = SharedSecret::new("client", TEST_VALID_SECRET);
            let identity_verifier = SharedSecret::new("client", TEST_VALID_SECRET);
            let mut client_mls = slim_mls::mls::Mls::new(
                name.clone(),
                identity_provider,
                identity_verifier,
                std::path::PathBuf::from("/tmp/client_mls"),
            );
            client_mls.initialize().unwrap();

            // Generate and save key package for server to use
            let key_package = client_mls.generate_key_package().unwrap();
            let key_package_path = format!("/tmp/mls_key_package_{}", group_identifier);
            std::fs::write(&key_package_path, &key_package).unwrap();
            info!("Client saved key package to: {}", key_package_path);

            // Wait for welcome message from server
            let welcome_path = format!("/tmp/mls_welcome_{}", group_identifier);
            info!("Client waiting for welcome message at: {}", welcome_path);
            let mut attempts = 0;
            let welcome_message = loop {
                if std::path::Path::new(&welcome_path).exists() {
                    let welcome_bytes = std::fs::read(&welcome_path).unwrap();
                    info!("Client found welcome message");
                    break welcome_bytes;
                }
                if attempts > 100 {
                    panic!("Timeout waiting for welcome message");
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                attempts += 1;
            };

            // Join the group
            let _group_id = client_mls.process_welcome(&welcome_message).unwrap();
            info!("Client successfully joined group");
        }

        // Get the session and spawn receiver for handling responses
        let session =
            spawn_session_receiver(session_ctx, local_name.to_string(), remote_app_name.clone());
        // let session = session_ctx.session_arc().unwrap();

        info!("Sending message to {}", remote_app_name);

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

                        // Use the extracted spawn_session_receiver function
                        let session = spawn_session_receiver(
                            session,
                            local_name.to_string(),
                            remote_app_name.clone(),
                        );

                        // Save the session
                        sessions.push(session);
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
        app.delete_session(&session).await.unwrap();
    }

    // consume the service and get the drain signal
    let signal = svc.signal();

    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => {}
        Err(_) => panic!("timeout waiting for drain for service"),
    }
}
