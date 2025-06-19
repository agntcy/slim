// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use slim_datapath::messages::{Agent, AgentType};
use tokio::time;
use tracing::info;

use slim::config;
use slim_service::{
    FireAndForgetConfiguration,
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
    let mut rx = svc
        .create_agent(&agent_name)
        .await
        .expect("failed to create agent");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    let local_agent_type = AgentType::from_strings("org", "default", local_agent);
    svc.subscribe(
        &agent_name,
        &local_agent_type,
        Some(agent_id),
        Some(conn_id),
    )
    .await
    .unwrap();

    // Set a route for the remote agent
    let route = AgentType::from_strings("org", "default", remote_agent);
    info!("allowing messages to remote agent: {:?}", route);
    svc.set_route(&agent_name, &route, None, conn_id)
        .await
        .unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // check what to do with the message
    if let Some(msg) = message {
        // create a fire and forget session
        let res = svc
            .create_session(
                &agent_name,
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
            )
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        // get the session
        let session = res.unwrap();

        // enable mls for the session
        slim_mls::add_mls_to_session(
            &svc,
            &agent_name,
            session.id,
            local_agent.to_string(),
            "/tmp/mls_identities",
        )
        .await
        .unwrap();

        // publish message
        svc.publish(&agent_name, session, &route, None, msg.into())
            .await
            .unwrap();
    }

    // wait for messages
    let mut messages = std::collections::VecDeque::<(String, session::Info)>::new();
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
                    svc.publish(&agent_name, msg.1, &route, None, msg.0.into())
                        .await
                        .unwrap();
                }
           }
            next = rx.recv() => {
                if next.is_none() {
                    break;
                }

                let session_msg = next.unwrap().expect("error");

                match &session_msg.message.message_type.unwrap() {
                    slim_datapath::api::ProtoPublishType(msg) => {
                        let payload = msg.get_payload();
                        info!(
                            "received message: {}",
                            std::str::from_utf8(&payload.blob).unwrap()
                        );
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
