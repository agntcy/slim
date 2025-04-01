// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::prelude::*;
use std::{collections::HashMap, sync::Arc};

use agp_datapath::messages::encoder::{encode_agent, encode_agent_type};
use agp_datapath::messages::utils::get_error;
use parking_lot::RwLock;
use testing::parse_line;
use tokio_util::sync::CancellationToken;

use agp_service::streaming::StreamingConfiguration;

use agp_gw::config;
use clap::Parser;
use indicatif::ProgressBar;
use tracing::{debug, error, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Workload input file, required if used in workload mode. If this is set the streaming mode is set to false.
    #[arg(short, long, value_name = "WORKLOAD", required = false)]
    workload: Option<String>,

    /// Runs in streaming mode.
    #[arg(
        short,
        long,
        value_name = "STREAMING",
        required = false,
        default_value_t = false
    )]
    streaming: bool,

    /// Agp config file
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
        value_name = "FREQUENCE",
        required = false,
        default_value_t = 0
    )]
    frequence: u32,

    /// used only in streaming mode, defines the maximum number of packets to send
    #[arg(short, long, value_name = "PACKETS", required = false)]
    max_packets: Option<u64>,
}

impl Args {
    pub fn msg_size(&self) -> &u32 {
        &self.msg_size
    }

    pub fn workload(&self) -> &Option<String> {
        &self.workload
    }

    pub fn streaming(&self) -> &bool {
        &self.streaming
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

    pub fn frequence(&self) -> &u32 {
        &self.frequence
    }

    pub fn max_packets(&self) -> &Option<u64> {
        &self.max_packets
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let input = args.workload();
    let config_file = args.config();
    let msg_size = *args.msg_size();
    let id = *args.id();
    let frequence = *args.frequence();
    let mut streaming = *args.streaming();
    let max_packets = args.max_packets;
    if input.is_some() {
        streaming = false;
    }

    info!(
        "configuration -- workload file: {}, agent config {}, publisher id: {}, streaming mode: {}, msg size: {}",
        input.as_ref().unwrap_or(&"None".to_string()),
        config_file,
        id,
        streaming,
        msg_size,
    );

    // start local agent
    // get service
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local agent
    let agent_name = encode_agent("cisco", "default", "publisher", id);
    // required in streaming mode
    let dest_name = encode_agent_type("cisco", "default", "subscriber");
    _ = svc
        .create_agent(&agent_name)
        .expect("failed to create agent");

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();
    info!("remote connection id = {}", conn_id);

    // subscribe for local name
    match svc
        .subscribe(
            &agent_name,
            agent_name.agent_type(),
            agent_name.agent_id_option(),
            Some(conn_id),
        )
        .await
    {
        Ok(_) => {}
        Err(e) => {
            panic!("an error accoured while adding a subscription {}", e);
        }
    }

    svc.set_route(&agent_name, &dest_name, None, conn_id)
        .await
        .unwrap();

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // STREAMING MODE
    if streaming {
        // create streaming session
        let res = svc
            .create_session(
                &agent_name,
                agp_service::session::SessionConfig::Streaming(StreamingConfiguration {
                    source: agent_name.clone(),
                    max_retries: None,
                    timeout: None,
                }),
            )
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        // get the session
        let session_info = res.unwrap();
        //let session_id = session_info.id;

        for i in 0..max_packets.unwrap_or(u64::MAX) {
            let payload: Vec<u8> = vec![120; msg_size as usize]; // ASCII for 'x' = 120
            info!("publishing message {}", i);
            if svc
                .publish(
                    &agent_name,
                    session_info.clone(),
                    &dest_name,
                    None,
                    10, // the packet will be sent in broadcast on dest_name
                    payload,
                )
                .await
                .is_err()
            {
                error!("an error occurred sending publication, the test will fail",);
            }
            if frequence != 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(frequence as u64)).await;
            }
        }
        return;
    }

    // WORKLOAD MODE
    // setup agent config
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

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

    // start local agent
    // get service
    let svc_id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local agent
    let agent_name = encode_agent("cisco", "default", "publisher", id);
    let mut rx = svc
        .create_agent(&agent_name)
        .expect("failed to create agent");

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();
    info!("remote connection id = {}", conn_id);

    // subscribe for local name
    match svc
        .subscribe(
            &agent_name,
            agent_name.agent_type(),
            agent_name.agent_id_option(),
            Some(conn_id),
        )
        .await
    {
        Ok(_) => {}
        Err(e) => {
            panic!("an error accoured while adding a subscription {}", e);
        }
    }

    // set routes for all subscriptions
    for r in routes {
        match svc
            .set_route(&agent_name, r.agent_type(), r.agent_id_option(), conn_id)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a route {}", e);
            }
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // create fire and forget session
    // create a fire and forget session
    let res = svc
        .create_session(
            &agent_name,
            agp_service::session::SessionConfig::FireAndForget(
                agp_service::FireAndForgetConfiguration {},
            ),
        )
        .await;
    if res.is_err() {
        panic!("error creating fire and forget session");
    }

    // get the session
    let session_info = res.unwrap();
    let session_id = session_info.id;

    // start receiving loop
    let results_list = Arc::new(RwLock::new(HashMap::new()));
    let clone_results_list = results_list.clone();
    let token = CancellationToken::new();
    let token_clone = token.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                res = rx.recv() => {
                    match res {
                        None => {
                            info!(%conn_id, "end of stream");
                            break;
                        }
                        Some(msg_info) => {
                            if msg_info.is_err() {
                                error!("error receiving message");
                                continue;
                            }

                            let msg_info = msg_info.unwrap();
                            let msg = msg_info.message;

                            // make sure the session matches
                            if session_info.id != session_id {
                                panic!("wrong session id {}", session_info.id);
                            }

                            // checks if the message is an error
                            match get_error(&msg) {
                                Ok(err) => {
                                    if err.is_some() && err.unwrap() {
                                        error!("An error occurred processing a message");
                                        continue;
                                    }
                                }
                                Err(err) => {
                                    panic!("error processing received packet {}", err);
                                }
                            }
                            match &msg.message_type.unwrap() {
                                agp_datapath::pubsub::ProtoPublishType(msg) => {
                                    // parse payload and add info to the result list
                                    let payload = agp_datapath::messages::utils::get_payload(msg);
                                    // the payload needs to start with the publication id and the received id
                                    // so it must contain at least 18 bytes
                                    if payload.len() < 18 {
                                        panic!("error parsing message");
                                    }

                                    let pub_id = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                                    let recv_id = u64::from_be_bytes(payload[9..17].try_into().unwrap());
                                    debug!("recv msg {} from {} on puslihser {}", pub_id, recv_id, id);
                                    let mut lock = clone_results_list.write();
                                    //write[pub_id as usize] = recv_id;
                                    lock.insert(pub_id, recv_id);
                                }
                                t => {
                                    panic!("received unexpected message: {:?}", t);
                                }
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

        // send message
        // at the moment we have only one connection so we can use it to send all messages there
        // the match will be performed by the remote GW.
        let agent_id = p.1.agent_id();
        let name_id = if agent_id == 0 { None } else { Some(agent_id) };

        // for the moment we send the message in anycast
        // we need to test also the match_all function
        if svc
            .publish(
                &agent_name,
                session_info.clone(),
                p.1.agent_type(),
                name_id,
                1,
                payload,
            )
            .await
            .is_err()
        {
            error!(
                "an error occurred sending publication {}, the test will fail",
                p.0
            );
        }

        if !args.quite() {
            bar.inc(1);
        }

        if frequence != 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(frequence as u64)).await;
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
            error!("test failed, the number of publications received is different from the number of publications sent. sent {} received {}", oracle.len(), lock.len());
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
