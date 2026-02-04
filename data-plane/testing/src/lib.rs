// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod common;
pub mod utils;

use std::{num::ParseIntError, str::SplitWhitespace};

use slim_config::component::id::ID;
use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_datapath::messages::Name;
use slim_service::{Service, ServiceConfiguration};
use thiserror::Error;

/// Build a client-only Service configured to connect to a local dataplane port.
/// This helper is shared across test programs.
/// - port: dataplane server port
/// - service_id: component identifier string, e.g. "slim/0"
pub fn build_client_service(port: u16, service_id: &str) -> Service {
    let endpoint = format!("http://localhost:{}", port);
    let client_cfg = GrpcClientConfig::with_endpoint(&endpoint)
        .with_tls_setting(TlsClientConfig::default().with_insecure(true));
    let service_cfg = ServiceConfiguration::new().with_dataplane_client(vec![client_cfg]);
    let svc_id = ID::new_with_str(service_id).expect("invalid service id");

    service_cfg
        .build_server(svc_id)
        .expect("failed to build service")
}

#[derive(Error, Debug, PartialEq)]
pub enum ParsingError {
    #[error("parsing error - missing organization")]
    MissingOrganization,
    #[error("parsing error - missing namespace")]
    MissingNamespace,
    #[error("parsing error - missing app")]
    MissingApp,
    #[error("parsing error - missing id")]
    MissingId,
    #[error("parsing error - missing subscriber id")]
    MissingSubscriberId,
    #[error("parsing error - missing publication id")]
    MissingPublicationId,
    #[error("parsing error - missing type (PUB/SUB)")]
    MissingType,
    #[error("parsing error - unknown type (must be PUB or SUB)")]
    UnknownType,
    #[error("parse int error")]
    ParseIntError(#[from] ParseIntError),
    #[error("unexpected number of receivers: {actual}/{expected}")]
    UnexpectedNumberOfReceivers { expected: usize, actual: usize },
    #[error("end of workload")]
    EOWError,
    #[error("unknown error")]
    Unknown,
}

#[derive(Debug)]
pub struct ParsedMessage {
    /// message type (SUB or PUB)
    pub msg_type: String,

    /// name used to send the publication
    pub name: Name,

    /// publication id to add in the payload
    pub id: u64,

    /// list of possible receives for the publication
    pub receivers: Vec<u64>,
}

fn parse_ids(iter: &mut SplitWhitespace<'_>) -> Result<Name, ParsingError> {
    let org = iter
        .next()
        .ok_or(ParsingError::MissingOrganization)?
        .parse::<String>()
        .map_err(|_e| ParsingError::MissingOrganization)?;
    let namespace = iter
        .next()
        .ok_or(ParsingError::MissingNamespace)?
        .parse::<String>()
        .map_err(|_e| ParsingError::MissingNamespace)?;
    let app_val = iter
        .next()
        .ok_or(ParsingError::MissingApp)?
        .parse::<String>()
        .map_err(|_e| ParsingError::MissingApp)?;
    let id = iter
        .next()
        .ok_or(ParsingError::MissingId)?
        .parse::<u64>()
        .map_err(|_e| ParsingError::MissingId)?;

    Ok(Name::from_strings([org, namespace, app_val]).with_id(id))
}

pub fn parse_sub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    // this a valid subscription, skip subscription id
    iter.next();

    // get subscriber id
    match iter.next() {
        None => Err(ParsingError::MissingSubscriberId),
        Some(id_str) => {
            let x = id_str.parse()?;
            let sub = parse_ids(&mut iter)?;

            Ok(ParsedMessage {
                msg_type: "SUB".to_string(),
                name: sub,
                id: x,
                receivers: vec![],
            })
        }
    }
}

pub fn parse_pub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    // this a valid publication, get pub id
    let id = iter
        .next()
        .ok_or(ParsingError::MissingPublicationId)?
        .parse::<u64>()?;

    // get the publication name
    let pub_name = parse_ids(&mut iter)?;

    // get the len of the possible receivers
    let size = iter.next().unwrap().parse::<u64>()?;

    // collect the list of possible receivers
    let mut receivers = vec![];
    for recv in iter {
        recv.parse::<u64>().map(|x| receivers.push(x))?;
    }

    if receivers.len() != size as usize {
        return Err(ParsingError::UnexpectedNumberOfReceivers {
            expected: size as usize,
            actual: receivers.len(),
        });
    }

    Ok(ParsedMessage {
        msg_type: "PUB".to_string(),
        name: pub_name,
        id,
        receivers,
    })
}

pub fn parse_line(line: &str) -> Result<ParsedMessage, ParsingError> {
    let mut iter = line.split_whitespace();
    let msg_type = iter.next().ok_or(ParsingError::MissingType)?.to_string();

    match msg_type.as_str() {
        "SUB" => parse_sub(iter),
        "PUB" => parse_pub(iter),
        _ => Err(ParsingError::UnknownType),
    }
}
