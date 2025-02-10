// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tracing::info;

use agp_gw::config;
use agp_pubsub_proto::messages::encoder::{encode_agent_class, encode_agent_from_string};

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

    info!(%config_file, %local_agent, %remote_agent, "starting client");

    // get service
    let id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let svc = config.services.get_mut(&id).unwrap();

    // create local agent
    let agent_name = encode_agent_from_string("cisco", "default", local_agent, 0);
    let mut rx = svc.create_agent(agent_name.clone());

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();

    // Set a route for the remote agent
    let route = encode_agent_class("cisco", "default", remote_agent);
    info!("allowing messages to remote agent: {:?}", route);
    svc.set_route(&route, None, conn_id).await.unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // check what to do with the message
    if let Some(msg) = message {
        // publish message
        svc.publish(&route, None, 1, msg.into()).await.unwrap();
    }

    // wait for messages
    loop {
        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.message_type.unwrap() {
            agp_pubsub_proto::ProtoPublishType(msg) => {
                let payload = agp_pubsub_proto::messages::utils::get_payload(msg);
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

        // send a message back
        let msg = "hello from the other side";
        svc.publish(&route, None, 1, msg.into()).await.unwrap();

        // sleep
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
