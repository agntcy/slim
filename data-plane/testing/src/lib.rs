// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod utils;

use std::str::SplitWhitespace;

use slim_datapath::messages::Name;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ParsingError {
    #[error("parsing error {0}")]
    ParsingError(String),
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
        .ok_or(ParsingError::ParsingError(
            "missing organization".to_string(),
        ))?
        .parse::<String>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse organization: {}", e)))?;
    let namespace = iter
        .next()
        .ok_or(ParsingError::ParsingError("missing namespace".to_string()))?
        .parse::<String>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse namespace: {}", e)))?;
    let app_val = iter
        .next()
        .ok_or(ParsingError::ParsingError("missing app".to_string()))?
        .parse::<String>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse app name: {}", e)))?;
    let id = iter
        .next()
        .ok_or(ParsingError::ParsingError("missing id".to_string()))?
        .parse::<u64>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse app id: {}", e)))?;

    Ok(Name::from_strings([org, namespace, app_val]).with_id(id))
}

pub fn parse_sub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    // this a valid subscription, skip subscription id
    iter.next();

    // get subscriber id
    match iter.next() {
        None => Err(ParsingError::ParsingError(
            "missing subscriber id".to_string(),
        )),
        Some(id_str) => match id_str.parse::<u64>() {
            Ok(x) => {
                let sub = parse_ids(&mut iter)?;

                Ok(ParsedMessage {
                    msg_type: "SUB".to_string(),
                    name: sub,
                    id: x,
                    receivers: vec![],
                })
            }
            Err(e) => Err(ParsingError::ParsingError(e.to_string())),
        },
    }
}

pub fn parse_pub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    // this a valid publication, get pub id
    let id = iter
        .next()
        .ok_or_else(|| ParsingError::ParsingError("missing publication id".to_string()))?
        .parse::<u64>()
        .map_err(|e| {
            ParsingError::ParsingError(format!("failed to parse publication id: {}", e))
        })?;

    // get the publication name
    let pub_name = parse_ids(&mut iter)?;

    // get the len of the possible receivers
    let size = match iter.next().unwrap().parse::<u64>() {
        Ok(x) => x,
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    };

    // collect the list of possible receivers
    let mut receivers = vec![];
    for recv in iter {
        recv.parse::<u64>()
            .map(|x| receivers.push(x))
            .map_err(|e| {
                ParsingError::ParsingError(format!("failed to parse receiver id: {}", e))
            })?;
    }

    if receivers.len() != size as usize {
        return Err(ParsingError::ParsingError(format!(
            "expected {} receivers, got {}",
            size,
            receivers.len()
        )));
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
    let msg_type = iter
        .next()
        .ok_or_else(|| ParsingError::ParsingError("missing type".to_string()))?
        .to_string();

    match msg_type.as_str() {
        "SUB" => parse_sub(iter),
        "PUB" => parse_pub(iter),
        _ => Err(ParsingError::ParsingError(format!(
            "unknown type: {}",
            msg_type
        ))),
    }
}
