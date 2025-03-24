// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tokio::time;
use tracing::info;

use agp_datapath::messages::encoder::{encode_agent, encode_agent_type};
use agp_gw::config;
use agp_service::session::SessionType;

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
    let id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // create local agent
    let agent_id = 0;
    let agent_name = encode_agent("cisco", "default", local_agent, agent_id);
    let mut rx = svc
        .create_agent(&agent_name)
        .expect("failed to create agent");

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();
    info!("remote connection id = {}", conn_id);

    let local_agent_type = encode_agent_type("cisco", "default", local_agent);
    svc.subscribe(
        &agent_name,
        &local_agent_type,
        Some(agent_id),
        Some(conn_id),
    )
    .await
    .unwrap();

    // Set a route for the remote agent
    let route = encode_agent_type("cisco", "default", remote_agent);
    info!("allowing messages to remote agent: {:?}", route);
    svc.set_route(&agent_name, &route, None, conn_id)
        .await
        .unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let mut session = u32::MAX;

    // check what to do with the message
    if let Some(msg) = message {
        // create a fire and forget session
        let res = svc
            .create_session(&agent_name, SessionType::FireAndForget)
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        // get the session
        session = res.unwrap();

        // publish message
        svc.publish(&agent_name, session, &route, None, 1, msg.into())
            .await
            .unwrap();
    }

    // wait for messages
    let mut messages = std::collections::VecDeque::<String>::new();
    loop {
        tokio::select! {
            _ = agp_signal::shutdown() => {
                info!("Received shutdown signal");
                break;
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                // send a message back
                let msg = messages.pop_front();
                if let Some(msg) = msg {
                    svc.publish(&agent_name, session, &route, None, 1, msg.into())
                        .await
                        .unwrap();
                }
           }
            next = rx.recv() => {
                if next.is_none() {
                    break;
                }

                let (msg, session_info) = next.unwrap();

                // make sure the session is the same
                if session != u32::MAX && session_info.id != session {
                    panic!("wrong session id {}", session_info.id);
                } else {
                    session = session_info.id;
                }

                match &msg.message_type.unwrap() {
                    agp_datapath::pubsub::ProtoPublishType(msg) => {
                        let payload = agp_datapath::messages::utils::get_payload(msg);
                        info!(
                            "received message: {}",
                            std::str::from_utf8(payload).unwrap()
                        );
                    }
                    t => {
                        info!("received wrong message: {:?}", t);
                        break;
                    }
                }

                let msg = format!("hello from the {}", local_agent);
                messages.push_back(msg);
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
