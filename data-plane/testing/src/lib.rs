use std::str::SplitWhitespace;

use agp_datapath::messages::{Agent, AgentType};
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

    let mut a_type = AgentType::default();
    let mut sub = Agent::default();
    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_organization(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_namespace(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_agent_type(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            sub = sub.with_agent_id(x);
            sub = sub.with_agent_type(a_type);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

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
    let mut a_type = AgentType::default();
    let mut pub_name = Agent::default();
    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_organization(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_namespace(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            a_type = a_type.with_agent_type(x);
        }
        Err(e) => {
            return Err(ParsingError::ParsingError(e.to_string()));
        }
    }

    match iter.next().unwrap().parse::<u64>() {
        Ok(x) => {
            pub_name = pub_name.with_agent_id(x);
            pub_name = pub_name.with_agent_type(a_type);
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

    publication.name = pub_name;
    Ok(publication)
}

pub fn parse_line(line: &str) -> Result<ParsedMessage, ParsingError> {
    let mut iter = line.split_whitespace();
    let prefix = iter.next();

    if prefix == Some("SUB") {
        return parse_sub(iter);
    }

    if prefix == Some("PUB") {
        return parse_pub(iter);
    }

    // unable to parse this line
    Err(ParsingError::ParsingError("unknown prefix".to_string()))
}
