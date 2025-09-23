// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::prelude::*;
use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;
use slim_datapath::messages::Name;
use testing::parse_line;
use tokio_util::sync::CancellationToken;

use clap::Parser;
use indicatif::ProgressBar;
use tracing::{debug, error, info};

use slim::config;
use slim_auth::shared_secret::SharedSecret;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Workload input file, required if used in workload mode. If this is set the multicast mode is set to false.
    #[arg(short, long, value_name = "WORKLOAD", required = false)]
    workload: Option<String>,

    /// Slim config file
    #[arg(short, long, value_name = "CONFIGURATION", required = true)]
    config: String,

    /// Publisher id
    #[arg(short, long, value_name = "ID", required = true)]
    id: u64,

    /// Publication message size
    #[arg(
        short,
        long,
        value_name = "SIZE",
        required = false,
        default_value_t = 1500
    )]
    msg_size: u32,

    /// Runs in quite mode without progress bars
    #[arg(
        short,
        long,
        value_name = "QUITE",
        required = false,
        default_value_t = false
    )]
    quite: bool,

    /// time between publications in milliseconds
    #[arg(
        short,
        long,
        value_name = "FREQUENCY",
        required = false,
        default_value_t = 0
    )]
    frequency: u32,
}

impl Args {
    pub fn msg_size(&self) -> &u32 {
        &self.msg_size
    }

    pub fn workload(&self) -> &Option<String> {
        &self.workload
    }

    pub fn id(&self) -> &u64 {
        &self.id
    }

    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn quite(&self) -> &bool {
        &self.quite
    }

    pub fn frequency(&self) -> &u32 {
        &self.frequency
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let input = args.workload();
    let config_file = args.config();
    let msg_size = *args.msg_size();
    let id = *args.id();
    let frequency = *args.frequency();

    info!(
        "configuration -- workload file: {}, config {}, publisher id: {}, msg size: {}",
        input.as_ref().unwrap_or(&"None".to_string()),
        config_file,
        id,
        msg_size,
    );

    // start local app
    // get service
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local app
    let app_name = Name::from_strings(["agntcy", "default", "publisher"]).with_id(id);

    let (app, _rx) = svc
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
    info!("remote connection id = {}", conn_id);

    // subscribe for local name
    match app.subscribe(&app_name, Some(conn_id)).await {
        Ok(_) => {}
        Err(e) => {
            panic!("an error accoured while adding a subscription {}", e);
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // setup app config
    let mut publication_list = HashMap::new();
    let mut oracle = HashMap::new();
    let mut routes = Vec::new();

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

    info!("loading publications");
    for line in buf.lines() {
        match parse_line(line) {
            Ok(parsed_msg) => {
                if parsed_msg.msg_type == "SUB" {
                    routes.push(parsed_msg.name);
                } else if parsed_msg.msg_type == "PUB" {
                    // add pub to the publication_list
                    publication_list.insert(parsed_msg.id, parsed_msg.name);
                    // add receivers list to the oracle
                    oracle.insert(parsed_msg.id, parsed_msg.receivers);
                }
            }
            Err(e) => {
                panic!("error while parsing the workload file: {}", e);
            }
        }
    }

    // set routes for all subscriptions
    for r in routes {
        match app.set_route(&r, conn_id).await {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a route {}", e);
            }
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // create a point to point session
    let res = app
        .create_session(
            slim_service::session::SessionConfig::PointToPoint(
                slim_service::PointToPointConfiguration::default(),
            ),
            None,
        )
        .await;
    if res.is_err() {
        panic!("error creating point to point session");
    }

    // get the session
    let session_ctx = res.unwrap();
    let session_id = session_ctx.session.upgrade().unwrap().id();

    // start receiving loop
    let results_list = Arc::new(RwLock::new(HashMap::new()));
    let clone_results_list = results_list.clone();
    let token = CancellationToken::new();
    let token_clone = token.clone();

    // Clone the Arc to session for later use
    let session_arc = session_ctx.session_arc().unwrap();

    session_ctx.spawn_receiver(move |mut rx, _weak, _meta| async move {
        loop {
            tokio::select! {
                res = rx.recv() => {
                    match res {
                        None => {
                            info!(%conn_id, "end of stream");
                            break;
                        }
                        Some(message) => {
                            if message.is_err() {
                                error!("error receiving message");
                                continue;
                            }
                            let msg = message.unwrap();
                                    if msg.get_session_header().get_session_id() != session_id { continue; }
                                    if let Some(slim_datapath::api::ProtoPublishType(publish)) = msg.message_type.as_ref() {
                                        let payload = &publish.get_payload().blob;
                                        if payload.len() < 18 { panic!("error parsing message"); }
                                        let pub_id = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                                        let recv_id = u64::from_be_bytes(payload[9..17].try_into().unwrap());
                                        debug!("recv msg {} from {} on publisher {}", pub_id, recv_id, id);
                                        let mut lock = clone_results_list.write();
                                        lock.insert(pub_id, recv_id);
                                    }
                            }
                        }
                    }

                _ = token_clone.cancelled() => {
                    info!("shutting down receiving thread");
                    break;
                }
            }
        }
    });

    // send all the subscription in the list
    info!("configuration completed, start test");
    let bar = ProgressBar::new(publication_list.len() as u64);
    if *args.quite() {
        bar.finish_and_clear();
    }
    #[allow(clippy::disallowed_types)]
    let start = std::time::Instant::now();
    for p in publication_list.iter() {
        // this is the payload of the message.
        // the first 8 bytes will be replaced by the pub id follow by 0x00
        let mut payload: Vec<u8> = vec![120; msg_size as usize]; // ASCII for 'x' = 120

        // update payload
        let pid = p.0.to_be_bytes().to_vec();
        for (i, v) in pid.iter().enumerate() {
            payload[i] = *v;
        }
        payload[pid.len()] = 0;

        // for the moment we send the message in anycast
        // we need to test also the match_all function
        if session_arc.publish(p.1, payload, None, None).await.is_err() {
            error!(
                "an error occurred sending publication {}, the test will fail",
                p.0
            );
        }

        if !args.quite() {
            bar.inc(1);
        }

        if frequency != 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(frequency as u64)).await;
        }
    }
    let duration = start.elapsed();

    if !args.quite() {
        bar.finish();
    }

    info!("sending time: {:?}", duration);

    // wait few seconds after send all publications
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // stop the receiving loop
    token.cancel();

    // wait few seconds for the recever thread to stop
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // check the test results
    info!("check test correctness");
    let bar = ProgressBar::new(oracle.len() as u64);
    if *args.quite() {
        bar.finish_and_clear();
    }
    let mut succeeded = true;
    // oracle and results_list must be of the same size
    {
        let lock = results_list.read();
        if oracle.len() != lock.len() {
            succeeded = false;
            error!(
                "test failed, the number of publications received is different from the number of publications sent. sent {} received {}",
                oracle.len(),
                lock.len()
            );
        }
    }
    for p in oracle.iter() {
        let lock = results_list.read();
        match lock.get(p.0) {
            None => {
                succeeded = false;
                error!("test failed, no reply received for publication {}", p.0);
            }
            Some(val) => {
                if !p.1.contains(val) {
                    succeeded = false;
                    error!(
                        "test failed, publication id {} received from the wrong subscriber id {}",
                        p.0, val
                    );
                }
            }
        };
        if !args.quite() {
            bar.inc(1);
        }
    }

    if !args.quite() {
        bar.finish();
    }

    if succeeded {
        info!("test succeeded");
    }
}
