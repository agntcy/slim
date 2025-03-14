// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use super::encoder::{Agent, AgentType, DEFAULT_AGENT_ID};
use crate::pubsub::{
    proto::pubsub::v1::ServiceHeaderType, AgpHeader, Content, ProtoAgent, ProtoMessage,
    ProtoPublish, ProtoPublishType, ProtoSubscribe, ProtoSubscribeType, ProtoUnsubscribe,
    ProtoUnsubscribeType, ServiceHeader,
};

use thiserror::Error;
use tracing::error;

#[derive(Error, Debug, PartialEq)]
pub enum MessageError {
    #[error("AGP header not found")]
    AgpHeaderNotFound,
    #[error("source not found")]
    SourceNotFound,
    #[error("destination not found")]
    DestinationNotFound,
    #[error("control header not found")]
    ControlHeaderNotFound,
}

// utils funtions for names
fn create_agent_name(name: &Agent) -> Option<ProtoAgent> {
    let mut id = None;
    if name.agent_id() != &DEFAULT_AGENT_ID {
        id = Some(*name.agent_id())
    }
    Some(ProtoAgent {
        organization: *name.agent_type().organization(),
        namespace: *name.agent_type().namespace(),
        agent_type: *name.agent_type().agent_type(),
        agent_id: id,
    })
}

// utils functions for agp header
pub fn create_agp_header(
    source: &Agent,
    destination: &Agent,
    recv_from: Option<u64>,
    forward_to: Option<u64>,
    incoming_conn: Option<u64>,
    error: Option<bool>,
) -> Option<AgpHeader> {
    Some(AgpHeader {
        source: create_agent_name(source),
        destination: create_agent_name(destination),
        recv_from,
        forward_to,
        incoming_conn,
        error,
    })
}

fn get_agp_header(msg: &ProtoMessage) -> Option<AgpHeader> {
    let header = match &msg.message_type {
        Some(msg_type) => match msg_type {
            ProtoPublishType(publish) => publish.header,
            ProtoSubscribeType(sub) => sub.header,
            ProtoUnsubscribeType(unsub) => unsub.header,
        },
        None => None,
    };

    header
}

// clear header
pub fn clear_agp_header(msg: &ProtoMessage) -> Result<(), MessageError> {
    match get_agp_header(msg) {
        Some(mut header) => {
            header.recv_from = None;
            header.forward_to = None;
            header.incoming_conn = None;
            header.error = None;
            Ok(())
        }
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

// set incoming connection
pub fn set_incoming_connection(
    msg: &ProtoMessage,
    incoming_conn: Option<u64>,
) -> Result<(), MessageError> {
    match get_agp_header(msg) {
        Some(mut header) => {
            header.incoming_conn = incoming_conn;
            Ok(())
        }
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

// agp header getters
pub fn get_recv_from(msg: &ProtoMessage) -> Result<Option<u64>, MessageError> {
    match get_agp_header(msg) {
        Some(header) => Ok(header.recv_from),
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

pub fn get_forward_to(msg: &ProtoMessage) -> Result<Option<u64>, MessageError> {
    match get_agp_header(msg) {
        Some(header) => Ok(header.forward_to),
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

fn get_incoming_conn(msg: &ProtoMessage) -> Result<Option<u64>, MessageError> {
    match get_agp_header(msg) {
        Some(header) => Ok(header.incoming_conn),
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

fn get_error(msg: &ProtoMessage) -> Result<Option<bool>, MessageError> {
    match get_agp_header(msg) {
        Some(header) => Ok(header.error),
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

pub fn get_source(msg: &ProtoMessage) -> Result<(AgentType, Option<u64>), MessageError> {
    match get_agp_header(msg) {
        Some(header) => match header.source {
            Some(source) => {
                let agent_type =
                    AgentType::new(source.organization, source.namespace, source.agent_type);
                Ok((agent_type, source.agent_id))
            }
            None => Err(MessageError::SourceNotFound),
        },
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

pub fn get_name(msg: &ProtoMessage) -> Result<(AgentType, Option<u64>), MessageError> {
    match get_agp_header(msg) {
        Some(header) => match header.destination {
            Some(destination) => {
                let agent_type = AgentType::new(
                    destination.organization,
                    destination.namespace,
                    destination.agent_type,
                );
                Ok((agent_type, destination.agent_id))
            }
            None => Err(MessageError::DestinationNotFound),
        },
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

// utils functions for service header
fn create_service_header(
    header_type: i32,
    id: u32,
    stream: Option<u32>,
    rtx: Option<u32>,
) -> Option<ServiceHeader> {
    Some(ServiceHeader {
        header_type,
        id,
        stream,
        rtx,
    })
}

pub fn create_default_service_header() -> Option<ServiceHeader> {
    create_service_header(ServiceHeaderType::CtrlFnf.into(), 0, None, None)
}

// getters for service header
fn get_msg_id(msg: &ProtoPublish) -> Result<u32, MessageError> {
    match msg.control {
        Some(header) => Ok(header.id),
        None => Err(MessageError::ControlHeaderNotFound),
    }
}

fn get_stream_id(msg: &ProtoPublish) -> Result<Option<u32>, MessageError> {
    match msg.control {
        Some(header) => Ok(header.stream),
        None => Err(MessageError::ControlHeaderNotFound),
    }
}

fn get_rtx_id(msg: &ProtoPublish) -> Result<Option<u32>, MessageError> {
    match msg.control {
        Some(header) => Ok(header.rtx),
        None => Err(MessageError::ControlHeaderNotFound),
    }
}

// utils functions for messages
pub fn create_subscription(
    header: Option<AgpHeader>,
    metadata: HashMap<String, String>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoSubscribeType(ProtoSubscribe { header })),
    }
}

fn create_subscription_from(name: &Agent, recv_from: u64) -> ProtoMessage {
    // this message is used to set the state inside the local subscription table.
    // it emulates the reception of a subscription message from a remote end point through
    // the connection recv_from
    // this allows to forward pub messages using a standard match on the subscription tables
    // it works in a similar way to the set_route command in IP: it creates a route to a destion
    // through a local interface

    // the source field is not used in this case, set it to default
    let source = Agent::default();
    let header = create_agp_header(&source, name, Some(recv_from), None, None, None);

    // create a subscription with the recv_from field in the header
    // the result is that the subscription will be added to the local
    // subscription table with connection = recv_from
    create_subscription(header, HashMap::new())
}

fn create_subscription_to_forward(source: &Agent, name: &Agent, forward_to: u64) -> ProtoMessage {
    // this subscription can be received only from a local connection
    // when this message is received the subscription is set in the local table
    // and forwarded to the connection forward_to to set the subscription remotely
    // before forward the subscription the forward_to needs to be set to None

    let header = create_agp_header(source, name, None, Some(forward_to), None, None);
    create_subscription(header, HashMap::new())
}

fn create_unsubscription(
    header: Option<AgpHeader>,
    metadata: HashMap<String, String>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoUnsubscribeType(ProtoUnsubscribe { header })),
    }
}

fn create_unsubscription_from(name: &Agent, recv_from: u64) -> ProtoMessage {
    // same as subscription from but it removes the state

    // the source field is not used in this case, set it to default
    let source = Agent::default();
    let header = create_agp_header(&source, name, Some(recv_from), None, None, None);

    // create the unsubscription with the metadata
    create_unsubscription(header, HashMap::new())
}

fn create_unsubscription_to_forward(source: &Agent, name: &Agent, forward_to: u64) -> ProtoMessage {
    // same as subscription to forward but it removes the state

    let header = create_agp_header(source, name, None, Some(forward_to), None, None);
    create_unsubscription(header, HashMap::new())
}

pub fn create_publication(
    header: Option<AgpHeader>,
    control: Option<ServiceHeader>,
    metadata: HashMap<String, String>,
    fanout: u32,
    content_type: &str,
    blob: Vec<u8>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoPublishType(ProtoPublish {
            header,
            control,
            fanout,
            msg: Some(Content {
                content_type: content_type.to_string(),
                blob,
            }),
        })),
    }
}

pub fn get_fanout(msg: &ProtoPublish) -> u32 {
    msg.fanout
}

pub fn get_payload(msg: &ProtoPublish) -> &[u8] {
    &msg.msg.as_ref().unwrap().blob
}
