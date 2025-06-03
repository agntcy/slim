// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::SplitWhitespace;

use slim_datapath::messages::{Agent, AgentType};
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

#[derive(Debug, Default)]
pub struct ParsedMessage {
    /// message type (SUB or PUB)
    pub msg_type: String,

    /// name used to send the publication
    pub name: Agent,

    /// publication id to add in the payload
    pub id: u64,

    /// list of possible receives for the publication
    pub receivers: Vec<u64>,
}

fn parse_ids(iter: &mut SplitWhitespace<'_>) -> Result<Agent, ParsingError> {
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
    let agent_type_val = iter
        .next()
        .ok_or(ParsingError::ParsingError("missing agent_type".to_string()))?
        .parse::<String>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse agent type: {}", e)))?;
    let agent_id = iter
        .next()
        .ok_or(ParsingError::ParsingError("missing agent_id".to_string()))?
        .parse::<u64>()
        .map_err(|e| ParsingError::ParsingError(format!("failed to parse agent id: {}", e)))?;

    let a_type = AgentType::from_strings(&org, &namespace, &agent_type_val);

    Ok(Agent::new(a_type, agent_id))
}

pub fn parse_sub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    let mut subscription = ParsedMessage {
        msg_type: "SUB".to_string(),
        ..Default::default()
    };

    // this a valid subscription, skip subscription id
    iter.next();

    // get subscriber id
    match iter.next() {
        None => {
            return Err(ParsingError::ParsingError(
                "missing subscriber id".to_string(),
            ));
        }
        Some(id_str) => match id_str.parse::<u64>() {
            Ok(x) => {
                subscription.id = x;
            }
            Err(e) => {
                return Err(ParsingError::ParsingError(e.to_string()));
            }
        },
    }

    let sub = parse_ids(&mut iter)?;

    subscription.name = sub;
    Ok(subscription)
}

pub fn parse_pub(mut iter: SplitWhitespace<'_>) -> Result<ParsedMessage, ParsingError> {
    let mut publication = ParsedMessage {
        msg_type: "PUB".to_string(),
        ..Default::default()
    };

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
    let pub_name = parse_ids(&mut iter)?;

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

    publication.name = pub_name;
    Ok(publication)
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
