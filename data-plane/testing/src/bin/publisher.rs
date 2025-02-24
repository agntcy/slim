// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::prelude::*;
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;

use agp_datapath::messages::Agent;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use agp_datapath::messages::encoder::encode_agent_from_string;
use agp_gw::config;
use clap::Parser;
use indicatif::ProgressBar;
use tracing::{debug, error, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Workload input file
    #[arg(short, long, value_name = "WORKLOAD", required = true)]
    workload: String,

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
}

impl Args {
    pub fn msg_size(&self) -> &u32 {
        &self.msg_size
    }

    pub fn workload(&self) -> &String {
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
}

#[derive(Error, Debug, PartialEq)]
pub enum ParsingError {
    #[error("parsing error {0}")]
    ParsingError(String),
    #[error("end of subscriptions")]
    EOSError,
    #[error("unknown error")]
    Unknown,
}

#[derive(Debug, Default)]
struct Publication {
    /// name used to send the publication
    name: Agent,

    /// publication id to add in the payload
    id: u64,

    /// list of possible receives for the publication
    receivers: Vec<u64>,
}

fn parse_line(line: &str) -> Result<Option<Publication>, ParsingError> {
    let mut iter = line.split_whitespace();
    let prefix = iter.next();
    if prefix == Some("SUB") {
        // skip this line
        return Ok(None);
    }

    if prefix != Some("PUB") {
        // unable to parse this line
        return Err(ParsingError::ParsingError("unknown prefix".to_string()));
    }

    let mut publication = Publication::default();

    // this a valid publication, get pub id
    match iter.next() {
        None => {
            // unable to parse this line
            return Err(ParsingError::ParsingError(
                "missing publication id".to_string(),
            ));
        }
        Some(x_str) => match x_str.parse::<u64>() {
            Ok(x) => publication.id = x,
            Err(e) => {
                return Err(ParsingError::ParsingError(e.to_string()));
            }
        },
    }

    // get the publication name
    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            publication.name.agent_class.organization = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            publication.name.agent_class.namespace = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            publication.name.agent_class.agent_class = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            publication.name.agent_id = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    // get the len of the possible receivers
    let size = match iter.next().unwrap().parse::<u64>() {
        Ok(x) => x,
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    };

    // collect the list of possible receivers
    for recv in iter {
        match recv.parse::<u64>() {
            Ok(x) => {
                publication.receivers.push(x);
            }
            Err(e) => {
                return Err(ParsingError::ParsingError(e.to_string()));
            }
        }
    }

    if size as usize != publication.receivers.len() {
        return Err(ParsingError::ParsingError(
            "missing receiver ids".to_string(),
        ));
    }

    Ok(Some(publication))
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let input = args.workload();
    let config_file = args.config();
    let msg_size = *args.msg_size();
    let id = *args.id();

    // setup agent config
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(
        "configuration -- input file: {}, agent config: {}, msg size: {}",
        input, config_file, msg_size
    );

    let mut publication_list = HashMap::new();
    let mut oracle = HashMap::new();

    let res = File::open(input);
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
            Ok(publication_opt) => match publication_opt {
                None => {}
                Some(p) => {
                    // add pub to the publication_list
                    publication_list.insert(p.id, p.name);
                    // add receivers list to the oracle
                    oracle.insert(p.id, p.receivers);
                }
            },
            Err(e) => {
                if e == ParsingError::EOSError {
                    // nothing left to parse
                    break;
                } else {
                    panic!("error while parsing the workload file {}", e);
                }
            }
        }
    }

    // start local agent
    // get service
    let svc_id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local agent
    let agent_name = encode_agent_from_string("cisco", "default", "publisher", id);
    let mut rx = svc.create_agent(agent_name.clone());

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();
    info!("remote connection id = {}", conn_id);

    // subscribe for local name
    match svc
        .subscribe(&agent_name.agent_class, Some(agent_name.agent_id), conn_id)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            panic!("an error accoured while adding a subscription {}", e);
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // start receiving loop
    //let results_list = Arc::new(RwLock::new(vec![999999999; publication_list.len()])); // init to a value !=0
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
                        Some(result) => match result {
                            Ok(msg) => {
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
                            Err(_) => {
                                info!(%conn_id, "connection dropped");
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
        let name_id = if p.1.agent_id == 0 {
            None
        } else {
            Some(p.1.agent_id)
        };

        // for the moment we send the message in anycast
        // we need to test also the match_all function
        svc.send_msg(&p.1.agent_class, name_id, 1, payload, conn_id)
            .await
            .unwrap();

        if !args.quite() {
            bar.inc(1);
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
