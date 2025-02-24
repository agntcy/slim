// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::prelude::*;
use thiserror::Error;

use agp_datapath::messages::Agent;

use agp_datapath::messages::encoder::encode_agent_from_string;
use agp_gw::config;
use clap::Parser;
use indicatif::ProgressBar;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Workload input file
    #[arg(short, long, value_name = "WORKLOAD", required = true)]
    workload: String,

    /// Agp configuration file
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

    pub fn workload(&self) -> &String {
        &self.workload
    }

    pub fn config(&self) -> &String {
        &self.config
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

fn parse_line(line: &str, id: u64) -> Result<Option<Agent>, ParsingError> {
    let mut iter = line.split_whitespace();
    let prefix = iter.next();
    if prefix == Some("PUB") {
        // no other subscription to read
        return Err(ParsingError::EOSError);
    }

    if prefix != Some("SUB") {
        // unable to parse this line
        return Err(ParsingError::ParsingError("unknown prefix".to_string()));
    }

    // this a valid subscription, skip subscription id
    iter.next();

    // check subscriber id
    if iter.next() != Some(&id.to_string()) {
        // skip this line of the workload
        return Ok(None);
    }

    // this subscription need to be parsed
    let mut sub = Agent::default();
    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            sub.agent_class.organization = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            sub.agent_class.namespace = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            sub.agent_class.agent_class = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            sub.agent_id = x;
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    Ok(Some(sub))
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let input = args.workload();
    let config_file = args.config();
    let id = *args.id();
    let id_bytes = id.to_be_bytes().to_vec();

    // setup agent config
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(
        "configuration -- workload file: {}, agent config {}, subscriber id: {}",
        input, config_file, id
    );

    let mut subscriptions_list = Vec::new();

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

    info!("loading subscriptios for subscriber {}", id);
    for line in buf.lines() {
        match parse_line(line, id) {
            Ok(agent) => match agent {
                None => {}
                Some(s) => {
                    subscriptions_list.push(s);
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
    let agent_name = encode_agent_from_string("cisco", "default", "subscriber", id);
    let mut rx = svc.create_agent(agent_name.clone());

    // connect to the remote gateway
    let conn_id = svc.connect(None).await.unwrap();
    info!("remote connection id = {}", conn_id);

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // send all the subscription in the list
    info!("register subscriptions for subscriber {}", id);
    let bar = ProgressBar::new(subscriptions_list.len() as u64);
    for s in subscriptions_list.iter() {
        match svc
            .subscribe(&s.agent_class, Some(s.agent_id), conn_id)
            .await
        {
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
        let recv_msg = rx.recv().await.unwrap().unwrap();
        let pub_id;
        let msg_len;
        let source_class;
        let source_id;
        match &recv_msg.message_type.unwrap() {
            agp_datapath::pubsub::ProtoPublishType(msg) => {
                let payload = agp_datapath::messages::utils::get_payload(msg);
                // the payload needs to start with the publication id, so it has to contain
                // at least 8 bytes
                msg_len = payload.len();
                if msg_len < 8 {
                    panic!("error parsing message, unexpected payload format");
                }
                pub_id = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                match agp_datapath::messages::utils::process_name(&msg.source) {
                    Err(e) => {
                        panic!("error parsing message {}", e);
                    }
                    Ok(x) => {
                        source_class = x;
                    }
                }
                match agp_datapath::messages::utils::get_agent_id(&msg.source) {
                    None => {
                        panic!("error parsing message: unable to get source id");
                    }
                    Some(x) => {
                        source_id = x;
                    }
                }
            }
            t => {
                panic!("received unexpected message: {:?}", t);
            }
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
        svc.send_msg(&source_class, Some(source_id), 1, out_vec, conn_id)
            .await
            .unwrap();
    }
}
