// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use core::fmt;
use std::collections::HashMap;

use super::encoder::{Agent, AgentClass, DEFAULT_AGENT_ID};
use crate::pubsub::{
    Content, ProtoAgentClass, ProtoAgentGroup, ProtoAgentId, ProtoMessage, ProtoPublish,
    ProtoPublishType, ProtoSubscribe, ProtoSubscribeType, ProtoUnsubscribe, ProtoUnsubscribeType,
};

use thiserror::Error;
use tracing::error;

#[derive(Error, Debug, PartialEq)]
pub enum MessageError {
    #[error("name not found")]
    NameNotFound,
    #[error("class not found")]
    ClassNotFound,
    #[error("group not found")]
    GroupNotFound,
}

pub enum MetadataType {
    ReceivedFrom,
    ForwardTo,
    IncomingConnection,
    Error,
    Unknown,
}

impl fmt::Display for MetadataType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MetadataType::ReceivedFrom => write!(f, "ReceivedFrom"),
            MetadataType::ForwardTo => write!(f, "ForwardTo"),
            MetadataType::IncomingConnection => write!(f, "IncomingConnection"),
            MetadataType::Error => write!(f, "Error"),
            MetadataType::Unknown => write!(f, "Unknown"),
        }
    }
}

fn create_source(source: &Agent) -> Option<ProtoAgentId> {
    Some(ProtoAgentId {
        class: Some(ProtoAgentClass {
            group: Some(ProtoAgentGroup {
                organization: source.agent_class.organization,
                namespace: source.agent_class.namespace,
            }),
            class: source.agent_class.agent_class,
        }),
        id: Some(source.agent_id),
    })
}

fn create_name(name: &AgentClass, id: Option<u64>) -> Option<ProtoAgentId> {
    Some(ProtoAgentId {
        class: Some(ProtoAgentClass {
            group: Some(ProtoAgentGroup {
                organization: name.organization,
                namespace: name.namespace,
            }),
            class: name.agent_class,
        }),
        id,
    })
}

pub fn create_subscription(
    source: &Agent,
    name: &AgentClass,
    id: Option<u64>,
    metadata: HashMap<String, String>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoSubscribeType(ProtoSubscribe {
            source: create_source(source),
            name: create_name(name, id),
        })),
    }
}

pub fn create_unsubscription(
    source: &Agent,
    name: &AgentClass,
    id: Option<u64>,
    metadata: HashMap<String, String>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoUnsubscribeType(ProtoUnsubscribe {
            source: create_source(source),
            name: create_name(name, id),
        })),
    }
}

pub fn create_publication(
    source: &Agent,
    name: &AgentClass,
    id: Option<u64>,
    metadata: HashMap<String, String>,
    fanout: u32,
    content_type: &str,
    blob: Vec<u8>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoPublishType(ProtoPublish {
            source: create_source(source),
            name: create_name(name, id),
            fanout,
            msg: Some(Content {
                content_type: content_type.to_string(),
                blob,
            }),
        })),
    }
}

pub fn create_subscription_from(
    name: &AgentClass,
    id: Option<u64>,
    from_conn: u64,
) -> ProtoMessage {
    // this message is used to set the state inside the local subscription table.
    // it emulates the reception of a subscription message from a remote end point through
    // the connection from_conn
    // this allows to forward pub messages using a standard match on the subscription tables
    // it works in a similar way to the set_route command in IP: it creates a route to a destion
    // through a local interface

    // the source field is not used in this case, set it to default
    let source = Agent::default();

    // add the from_conn in the hashmap of the message
    let mut metadata = HashMap::new();
    metadata.insert(
        MetadataType::ReceivedFrom.to_string(),
        from_conn.to_string(),
    );

    // create a subscription with the metadata
    // the result will be that the subscription will be added to the local
    // subscription table with connection = from_conn
    create_subscription(&source, name, id, metadata)
}

pub fn create_subscription_to_forward(
    source: &Agent,
    name: &AgentClass,
    id: Option<u64>,
    to_conn: u64,
) -> ProtoMessage {
    // this subscription can be received only from a local connection
    // when this message is received the subscription is set in the local table
    // and forwarded to the connection to_conn to set the subscription remotely
    // before forward the subscription the metadata map is cleaned up

    // add the from_conn in the hashmap of the message
    let mut metadata = HashMap::new();
    metadata.insert(MetadataType::ForwardTo.to_string(), to_conn.to_string());

    create_subscription(source, name, id, metadata)
}

pub fn create_unsubscription_from(
    name: &AgentClass,
    id: Option<u64>,
    from_conn: u64,
) -> ProtoMessage {
    // same as subscription from but it removes the state

    // the source field is not used in this case, set it to default
    let source = Agent::default();

    // add the from_conn in the hashmap of the message
    let mut metadata = HashMap::new();
    metadata.insert(
        MetadataType::ReceivedFrom.to_string(),
        from_conn.to_string(),
    );

    // create the unsubscription with the metadata
    create_unsubscription(&source, name, id, metadata)
}

pub fn add_incoming_connection(msg: &mut ProtoMessage, in_connection: u64) {
    msg.metadata.insert(
        MetadataType::IncomingConnection.to_string(),
        in_connection.to_string(),
    );
}

pub fn get_incoming_connection(msg: &ProtoMessage) -> Option<u64> {
    match msg
        .metadata
        .get(&MetadataType::IncomingConnection.to_string())
    {
        None => None,
        Some(conn) => conn.parse::<u64>().ok(),
    }
}

pub fn get_source(msg: &ProtoMessage) -> Option<Agent> {
    let source = match &msg.message_type {
        Some(msg_type) => match msg_type {
            ProtoPublishType(publish) => publish.source,
            ProtoSubscribeType(sub) => sub.source,
            ProtoUnsubscribeType(unsub) => unsub.source,
        },
        None => None,
    };

    source?;

    let (class, id) = match process_name(&source) {
        Ok(class) => (Some(class), get_agent_id(&source)),
        Err(_) => (None, None),
    };

    let unwrap_class = class?;

    let src_name = Agent {
        agent_class: unwrap_class,
        agent_id: id.unwrap_or(DEFAULT_AGENT_ID),
    };

    Some(src_name)
}

pub fn get_name(msg: &ProtoMessage) -> Option<Agent> {
    let name = match &msg.message_type {
        Some(msg_type) => match msg_type {
            ProtoPublishType(publish) => publish.name,
            ProtoSubscribeType(sub) => sub.name,
            ProtoUnsubscribeType(unsub) => unsub.name,
        },
        None => None,
    };

    name?;

    let (class, id) = match process_name(&name) {
        Ok(class) => (Some(class), get_agent_id(&name)),
        Err(_) => (None, None),
    };

    let unwrap_class = class?;

    let dst_name = Agent {
        agent_class: unwrap_class,
        agent_id: id.unwrap_or(DEFAULT_AGENT_ID),
    };

    Some(dst_name)
}

pub fn create_unsubscription_to_forward(
    source: &Agent,
    name: &AgentClass,
    id: Option<u64>,
    to_conn: u64,
) -> ProtoMessage {
    // same as the subscription to forward but it remove the state

    // add the from_conn in the hashmap of the message
    let mut metadata = HashMap::new();
    metadata.insert(MetadataType::ForwardTo.to_string(), to_conn.to_string());

    create_unsubscription(source, name, id, metadata)
}

pub fn process_name(name: &Option<ProtoAgentId>) -> Result<AgentClass, MessageError> {
    if name.is_none() {
        error! {"unable to parse message, name not found"};
        return Err(MessageError::NameNotFound);
    }

    let name = name.unwrap();
    if name.class.is_none() {
        error! {"unable to parse message, class not found"};
        return Err(MessageError::ClassNotFound);
    }

    let class = name.class.unwrap();

    if class.group.is_none() {
        error! {"unable to parse message, group not found"};
        return Err(MessageError::GroupNotFound);
    }

    let group = class.group.unwrap();

    let class = AgentClass {
        organization: group.organization,
        namespace: group.namespace,
        agent_class: class.class,
    };

    Ok(class)
}

pub fn get_agent_id(name: &Option<ProtoAgentId>) -> Option<u64> {
    match name {
        None => None,
        Some(n) => n.id,
    }
}

pub fn get_fanout(pubmsg: &ProtoPublish) -> u32 {
    pubmsg.fanout
}

pub fn get_payload(pubmsg: &ProtoPublish) -> &[u8] {
    &pubmsg.msg.as_ref().unwrap().blob
}
