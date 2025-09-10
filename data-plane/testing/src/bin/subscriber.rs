// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::multicast::MulticastConfiguration;
use std::fs::File;
use std::io::prelude::*;
use std::time::Duration;
use testing::parse_line;

use clap::Parser;
use indicatif::ProgressBar;
use slim::config;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Workload input file, required if used in workload mode. If this is set the multicast mode is set to false.
    #[arg(short, long, value_name = "WORKLOAD", required = false)]
    workload: Option<String>,

    /// Slim configuration file
    #[arg(short, long, value_name = "CONFIGURATION", required = true)]
    config: String,

    /// Subscriber id
    #[arg(short, long, value_name = "ID", required = true)]
    id: u64,
}

impl Args {
    pub fn id(&self) -> &u64 {
        &self.id
    }

    pub fn workload(&self) -> &Option<String> {
        &self.workload
    }

    pub fn config(&self) -> &String {
        &self.config
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let input = args.workload();
    let config_file = args.config();
    let id = *args.id();
    let id_bytes = id.to_be_bytes().to_vec();

    // setup app config
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(
        "configuration -- workload file: {}, config {}, subscriber id: {}",
        input.as_ref().unwrap_or(&"None".to_string()),
        config_file,
        id,
    );

    // start local app
    // get service
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local app
    let app_name = Name::from_strings(["agntcy", "default", "subscriber"]).with_id(id);
    let (app, mut rx) = svc
        .create_app(
            &app_name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
        )
        .await
        .expect("failed to create app");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    // run subscriber in workload mode
    let mut subscriptions_list = Vec::new();

    let res = File::open(input.as_ref().unwrap());
    if res.is_err() {
        panic!("error opening the input file");
    }

    let mut file = res.unwrap();

    let mut buf = String::new();
    let res = file.read_to_string(&mut buf);
    if res.is_err() {
        panic!("error reading the file");
    }

    info!("loading subscriptios for subscriber {}", id);
    for line in buf.lines() {
        match parse_line(line) {
            Ok(parsed_msg) => {
                if parsed_msg.msg_type == "SUB" && parsed_msg.id == id {
                    subscriptions_list.push(parsed_msg.name);
                } else if parsed_msg.msg_type == "PUB" {
                    // no more subscriptions to process, exit loop
                    break;
                }
            }
            Err(e) => {
                panic!("error while parsing the workload file {}", e);
            }
        }
    }

    let res = app
        .create_session(
            slim_service::session::SessionConfig::Multicast(MulticastConfiguration::new(
                app_name.clone(),
                false,
                Some(10),
                Some(Duration::from_millis(1000)),
                false,
            )),
            None,
        )
        .await;
    if res.is_err() {
        panic!("error creating fire and forget session");
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // send all the subscription in the list
    info!("register subscriptions for subscriber {}", id);
    let bar = ProgressBar::new(subscriptions_list.len() as u64);
    for s in subscriptions_list.iter() {
        match app.subscribe(s, Some(conn_id)).await {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a subscription {}", e);
            }
        }
        bar.inc(1);
    }
    bar.finish();

    info!("waiting for incoming messages");
    // wait for messages
    loop {
        let recv_msg = rx.recv().await.unwrap().expect("error");
        let pub_id;
        let msg_len;
        let source;
        match &recv_msg.message.message_type {
            None => {
                panic!("message type is missing");
            }
            Some(msg_type) => match msg_type {
                slim_datapath::api::ProtoPublishType(msg) => {
                    let payload = &msg.get_payload().blob;
                    // the payload needs to start with the publication id, so it has to contain
                    // at least 8 bytes
                    msg_len = payload.len();
                    if msg_len < 8 {
                        panic!("error parsing message, unexpected payload format");
                    }
                    pub_id = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                    source = recv_msg.message.get_source();
                }
                t => {
                    panic!("received unexpected message: {:?}", t);
                }
            },
        }

        // create a new message with the same len with the format
        // pub_id 0x00 id 0x00 payload(size = msg_len - 9)
        debug!("received pub {}, size {}", pub_id, msg_len);
        let mut out_vec = pub_id.to_be_bytes().to_vec();
        out_vec.push(0);
        for b in id_bytes.iter() {
            out_vec.push(*b);
        }
        out_vec.push(0);
        while out_vec.len() < msg_len {
            out_vec.push(120); //ASCII for 'x'
        }

        // send message
        app.publish_to(recv_msg.info, &source, conn_id, out_vec, None, None)
            .await
            .unwrap();
    }
}
