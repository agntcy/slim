// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_service::session::Notification;
use std::fs::File;
use std::io::prelude::*;
use testing::parse_line;

use clap::Parser;
use indicatif::ProgressBar;
use slim::config;
use tracing::{debug, error, info};

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
            SharedSecret::new("a", "test"),
            SharedSecret::new("a", "test"),
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

    info!("waiting for new session");
    loop {
        match rx.recv().await {
            Some(n) => match n {
                Ok(notification) => match notification {
                    Notification::NewSession(session_context) => {
                        let _ = spawn_session_receiver(session_context, conn_id, id_bytes.clone());
                    }
                    _ => {
                        error!("Unexpected notification type");
                        continue;
                    }
                },
                Err(_) => {
                    panic!("error receiving a new session");
                }
            },
            None => {
                error!("error while waiting for a new session, quit application");
                break;
            }
        }
    }
}

fn spawn_session_receiver(
    session_ctx: slim_service::session::context::SessionContext<SharedSecret, SharedSecret>,
    conn_id: u64,
    id_bytes: Vec<u8>,
) -> std::sync::Arc<slim_service::session::Session<SharedSecret, SharedSecret>> {
    session_ctx
        .spawn_receiver(move |mut rx, weak, _meta| async move {
            info!("session handler started");
            loop {
                match rx.recv().await {
                    Some(Ok(msg)) => {
                        debug!("received message in session handler");
                        if let Some(slim_datapath::api::ProtoPublishType(publish)) =
                            msg.message_type.as_ref()
                        {
                            let payload = &publish.get_payload().blob;
                            let msg_len = payload.len();
                            if msg_len < 8 {
                                panic!("error parsing message, unexpected payload format");
                            }
                            let pub_id = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                            let source = msg.get_source();
                            debug!("received pub {}, size {}", pub_id, msg_len);
                            let mut out_vec = pub_id.to_be_bytes().to_vec();
                            out_vec.push(0);
                            for b in id_bytes.iter() {
                                out_vec.push(*b);
                            }
                            out_vec.push(0);
                            while out_vec.len() < msg_len {
                                out_vec.push(120);
                            }
                            if let Some(session_arc) = weak.upgrade() {
                                session_arc
                                    .publish_to(&source, conn_id, out_vec, None, None)
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("error receiving message in session handler: {:?}", e);
                        continue;
                    }
                    None => {
                        error!("session handler channel closed");
                        break;
                    }
                }
            }
        })
        .upgrade()
        .unwrap()
}
