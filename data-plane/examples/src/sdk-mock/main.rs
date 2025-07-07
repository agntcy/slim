// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use slim_datapath::messages::{Agent, AgentType};
use std::sync::Arc;
use tokio::{sync::Mutex, time};
use tracing::info;

use slim::config;
use slim_auth::simple::Simple;
use slim_service::{
    FireAndForgetConfiguration,
    interceptor_mls::{self, MlsInterceptor},
    session::{self, SessionConfig},
};

mod args;

#[tokio::main]
async fn main() {
    // parse command line arguments
    let args = args::Args::parse();

    // get config file
    let config_file = args.config();

    // get local agent id
    let local_agent = args.local_agent();

    // get remote agent id
    let remote_agent = args.remote_agent();

    // get message
    let message = args.message();

    // get MLS group identifier
    let mls_group_id = args.mls_group_id();

    // create configured components
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, %local_agent, %remote_agent, "starting client");

    // get service
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // create local agent
    let agent_id = 0;
    let agent_name = Agent::from_strings("org", "default", local_agent, agent_id);
    let (app, mut rx) = svc
        .create_app(&agent_name, Simple::new("secret"), Simple::new("secret"))
        .await
        .expect("failed to create agent");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    let local_agent_type = AgentType::from_strings("org", "default", local_agent);
    app.subscribe(&local_agent_type, Some(agent_id), Some(conn_id))
        .await
        .unwrap();

    // Set a route for the remote agent
    let route = AgentType::from_strings("org", "default", remote_agent);
    info!("allowing messages to remote agent: {:?}", route);
    app.set_route(&route, None, conn_id).await.unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // MLS setup, only if mls_group_id is provided
    let server_mls_option = if let Some(group_identifier) = mls_group_id {
        info!("MLS enabled with group identifier: {}", group_identifier);

        //TODO(zkacsand): temporary file based key package exchange, until the session API is ready to support it
        // Clean up previous run files
        let key_package_path = format!("/tmp/mls_key_package_{}", group_identifier);
        let welcome_path = format!("/tmp/mls_welcome_{}", group_identifier);
        let _ = std::fs::remove_file(&key_package_path);
        let _ = std::fs::remove_file(&welcome_path);

        // Clean up MLS identity directories
        let identity_path = format!("/tmp/mls_identities_{}", local_agent);
        let _ = std::fs::remove_dir_all(&identity_path);

        if message.is_some() {
            // Client: will join group after server creates it
            None
        } else {
            // Server: create group and wait for client key package
            let identity_provider = Arc::new(
                slim_mls::identity::FileBasedIdentityProvider::new(&identity_path).unwrap(),
            );
            let mut server_mls =
                slim_mls::mls::Mls::new(local_agent.to_string(), identity_provider);
            server_mls.initialize().await.unwrap();

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
            let (_, welcome_message) = server_mls.add_member(&key_package).unwrap();

            // Save welcome message for client
            std::fs::write(&welcome_path, &welcome_message).unwrap();
            info!("Server saved welcome message to: {}", welcome_path);

            Some((server_mls, group_id))
        }
    } else {
        info!("MLS disabled - no group identifier provided");
        None
    };

    // check what to do with the message
    if let Some(msg) = message {
        // create a fire and forget session
        let res = app
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                None,
            )
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        // get the session
        let session = res.unwrap();

        // Client MLS setup, only if mls_group_id is provided
        if let Some(group_identifier) = mls_group_id {
            // Clean up MLS identity directories
            let identity_path = format!("/tmp/mls_identities_{}", local_agent);

            // Client: generate key package and wait for welcome message
            let identity_provider = Arc::new(
                slim_mls::identity::FileBasedIdentityProvider::new(&identity_path).unwrap(),
            );
            let mut client_mls =
                slim_mls::mls::Mls::new(local_agent.to_string(), identity_provider);
            client_mls.initialize().await.unwrap();

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

            // enable mls for the session with group_id
            let interceptor = MlsInterceptor::new(Arc::new(Mutex::new(client_mls)));
            app.add_interceptor(session.id, Arc::new(interceptor))
                .await
                .unwrap();
        }

        // publish message
        app.publish(session, &route, None, msg.into())
            .await
            .unwrap();
    }

    // wait for messages
    let mut messages = std::collections::VecDeque::<(String, session::Info)>::new();
    let mut server_session_created = false;
    let mut server_mls_for_session = server_mls_option;
    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
                break;
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                // send a message back
                let msg = messages.pop_front();
                if let Some(msg) = msg {
                    app.publish(msg.1, &route, None, msg.0.into())
                        .await
                        .unwrap();
                }
           }
            next = rx.recv() => {
                if next.is_none() {
                    break;
                }

                let session_msg = next.unwrap().expect("error");

                // Setup MLS session for server on first message
                if message.is_none() && !server_session_created && server_mls_for_session.is_some() {
                    let (mls, _group_id) = server_mls_for_session.take().unwrap();
                    let interceptor = interceptor_mls::MlsInterceptor::new(
                        Arc::new(Mutex::new(mls)),
                    );
                    app.add_interceptor(session_msg.info.id, Arc::new(interceptor))
                        .await
                        .unwrap();
                    server_session_created = true;
                    info!("Server setup MLS for session");
                }

                match &session_msg.message.message_type.unwrap() {
                    slim_datapath::api::ProtoPublishType(msg) => {
                        let payload = msg.get_payload();
                        match std::str::from_utf8(&payload.blob) {
                            Ok(text) => {
                                info!("received message: {}", text);
                            }
                            Err(_) => {
                                info!("received encrypted message: {} bytes", payload.blob.len());
                            }
                        }
                    }
                    t => {
                        info!("received wrong message: {:?}", t);
                        break;
                    }
                }

                let msg = format!("hello from the {}", local_agent);
                messages.push_back((msg, session_msg.info));
            }
        }
    }

    info!("sdk-mock shutting down");

    // consume the service and get the drain signal
    let signal = svc.signal();

    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => {}
        Err(_) => panic!("timeout waiting for drain for service"),
    }
}
