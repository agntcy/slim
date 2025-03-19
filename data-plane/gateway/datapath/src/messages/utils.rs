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

// utils functions for names
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

pub fn create_agent_from_type(agent_type: &AgentType, agent_id: Option<u64>) -> Option<ProtoAgent> {
    Some(ProtoAgent {
        organization: *agent_type.organization(),
        namespace: *agent_type.namespace(),
        agent_type: *agent_type.agent_type(),
        agent_id,
    })
}

pub fn message_type_to_str(msg: &ProtoMessage) -> &str {
    match &msg.message_type {
        Some(ProtoPublishType(_)) => "publish",
        Some(ProtoSubscribeType(_)) => "subscribe",
        Some(ProtoUnsubscribeType(_)) => "unsubscribe",
        None => "unknown",
    }
}

// utils functions for agp header
pub fn create_agp_header(
    source: &Agent,
    name_type: &AgentType,
    name_id: Option<u64>,
    recv_from: Option<u64>,
    forward_to: Option<u64>,
    incoming_conn: Option<u64>,
    error: Option<bool>,
) -> Option<AgpHeader> {
    Some(AgpHeader {
        source: create_agent_name(source),
        destination: create_agent_from_type(name_type, name_id),
        recv_from,
        forward_to,
        incoming_conn,
        error,
    })
}

fn get_agp_header(msg: &ProtoMessage) -> Option<AgpHeader> {
    match &msg.message_type {
        Some(msg_type) => match msg_type {
            ProtoPublishType(publish) => publish.header,
            ProtoSubscribeType(sub) => sub.header,
            ProtoUnsubscribeType(unsub) => unsub.header,
        },
        None => None,
    }
}

fn get_agp_header_as_mut(msg: &mut ProtoMessage) -> Option<&mut AgpHeader> {
    match &mut msg.message_type {
        Some(ProtoPublishType(publish)) => publish.header.as_mut(),
        Some(ProtoSubscribeType(sub)) => sub.header.as_mut(),
        Some(ProtoUnsubscribeType(unsub)) => unsub.header.as_mut(),
        None => None,
    }
}

// this function cleans all the AGP header fields except
// for incoming_conn which is set upon message reception
pub fn clear_agp_header(msg: &mut ProtoMessage) -> Result<(), MessageError> {
    match get_agp_header_as_mut(msg) {
        Some(header) => {
            header.recv_from = None;
            header.forward_to = None;
            header.error = None;
            Ok(())
        }
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

// set incoming connection
pub fn set_incoming_connection(
    msg: &mut ProtoMessage,
    incoming_conn: Option<u64>,
) -> Result<(), MessageError> {
    match get_agp_header_as_mut(msg) {
        Some(header) => {
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

pub fn get_incoming_connection(msg: &ProtoMessage) -> Result<Option<u64>, MessageError> {
    match get_agp_header(msg) {
        Some(header) => Ok(header.incoming_conn),
        None => Err(MessageError::AgpHeaderNotFound),
    }
}

pub fn get_error(msg: &ProtoMessage) -> Result<Option<bool>, MessageError> {
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
#[allow(dead_code)]
fn get_msg_id(msg: &ProtoPublish) -> Result<u32, MessageError> {
    match msg.control {
        Some(header) => Ok(header.id),
        None => Err(MessageError::ControlHeaderNotFound),
    }
}

#[allow(dead_code)]
fn get_stream_id(msg: &ProtoPublish) -> Result<Option<u32>, MessageError> {
    match msg.control {
        Some(header) => Ok(header.stream),
        None => Err(MessageError::ControlHeaderNotFound),
    }
}

#[allow(dead_code)]
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

pub fn create_subscription_from(
    agent_type: &AgentType,
    agent_id: Option<u64>,
    recv_from: u64,
) -> ProtoMessage {
    // this message is used to set the state inside the local subscription table.
    // it emulates the reception of a subscription message from a remote end point through
    // the connection recv_from
    // this allows to forward pub messages using a standard match on the subscription tables
    // it works in a similar way to the set_route command in IP: it creates a route to a destion
    // through a local interface

    // the source field is not used in this case, set it to default
    let source = Agent::default();
    let header = create_agp_header(
        &source,
        agent_type,
        agent_id,
        Some(recv_from),
        None,
        None,
        None,
    );

    // create a subscription with the recv_from field in the header
    // the result is that the subscription will be added to the local
    // subscription table with connection = recv_from
    create_subscription(header, HashMap::new())
}

pub fn create_subscription_to_forward(
    source: &Agent,
    agent_type: &AgentType,
    agent_id: Option<u64>,
    forward_to: u64,
) -> ProtoMessage {
    // this subscription can be received only from a local connection
    // when this message is received the subscription is set in the local table
    // and forwarded to the connection forward_to to set the subscription remotely
    // before forward the subscription the forward_to needs to be set to None

    let header = create_agp_header(
        source,
        agent_type,
        agent_id,
        None,
        Some(forward_to),
        None,
        None,
    );
    create_subscription(header, HashMap::new())
}

#[allow(dead_code)]
fn create_unsubscription(
    header: Option<AgpHeader>,
    metadata: HashMap<String, String>,
) -> ProtoMessage {
    ProtoMessage {
        metadata,
        message_type: Some(ProtoUnsubscribeType(ProtoUnsubscribe { header })),
    }
}

pub fn create_unsubscription_from(
    agent_type: &AgentType,
    agent_id: Option<u64>,
    recv_from: u64,
) -> ProtoMessage {
    // same as subscription from but it removes the state

    // the source field is not used in this case, set it to default
    let source = Agent::default();
    let header = create_agp_header(
        &source,
        agent_type,
        agent_id,
        Some(recv_from),
        None,
        None,
        None,
    );

    // create the unsubscription with the metadata
    create_unsubscription(header, HashMap::new())
}

pub fn create_unsubscription_to_forward(
    source: &Agent,
    agent_type: &AgentType,
    agent_id: Option<u64>,
    forward_to: u64,
) -> ProtoMessage {
    // same as subscription to forward but it removes the state

    let header = create_agp_header(
        source,
        agent_type,
        agent_id,
        None,
        Some(forward_to),
        None,
        None,
    );
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

pub fn create_error_publication(error: String) -> ProtoMessage {
    let default_name = Agent::default();
    let header = create_agp_header(
        &default_name,
        default_name.agent_type(),
        None,
        None,
        None,
        None,
        Some(true),
    );
    create_publication(
        header,
        create_default_service_header(),
        HashMap::new(),
        1,
        "",
        error.into_bytes(),
    )
}

pub fn get_fanout(msg: &ProtoPublish) -> u32 {
    msg.fanout
}

pub fn get_payload(msg: &ProtoPublish) -> &[u8] {
    &msg.msg.as_ref().unwrap().blob
}

#[cfg(test)]
mod tests {
    use crate::messages::encoder::{encode_agent, encode_agent_type};

    use super::*;

    #[test]
    fn test_utils() {
        // test subscription
        let source = encode_agent("org", "ns", "type", 1);
        let name = encode_agent_type("org", "ns", "type");
        let header = create_agp_header(&source, &name, Some(2), None, None, None, None);
        let sub = create_subscription(header, HashMap::new());

        assert_eq!(header, get_agp_header(&sub));
        assert_eq!(None, get_recv_from(&sub).unwrap());
        assert_eq!(None, get_forward_to(&sub).unwrap());
        assert_eq!(None, get_incoming_connection(&sub).unwrap());
        let (got_source, got_source_id) = get_source(&sub).unwrap();
        assert_eq!(*source.agent_type(), got_source);
        assert_eq!(Some(1), got_source_id);
        let (got_name, got_name_id) = get_name(&sub).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(Some(2), got_name_id);

        let header_from = create_agp_header(
            &Agent::default(),
            &name,
            Some(2),
            Some(50),
            None,
            None,
            None,
        );
        let sub_from = create_subscription_from(&name, Some(2), 50);

        assert_eq!(header_from, get_agp_header(&sub_from));
        assert_eq!(Some(50), get_recv_from(&sub_from).unwrap());
        assert_eq!(None, get_forward_to(&sub_from).unwrap());
        assert_eq!(None, get_incoming_connection(&sub_from).unwrap());
        let (got_source, got_source_id) = get_source(&sub_from).unwrap();
        assert_eq!(*Agent::default().agent_type(), got_source);
        assert_eq!(Agent::default().agent_id_option(), got_source_id);
        let (got_name, got_name_id) = get_name(&sub_from).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(Some(2), got_name_id);

        let header_fwd = create_agp_header(&source, &name, None, None, Some(30), None, None);
        let mut sub_fwd = create_subscription_to_forward(&source, &name, None, 30);

        assert_eq!(header_fwd, get_agp_header(&sub_fwd));
        assert_eq!(None, get_recv_from(&sub_fwd).unwrap());
        assert_eq!(Some(30), get_forward_to(&sub_fwd).unwrap());
        assert_eq!(None, get_incoming_connection(&sub_fwd).unwrap());
        let (got_source, got_source_id) = get_source(&sub_fwd).unwrap();
        assert_eq!(*source.agent_type(), got_source);
        assert_eq!(Some(1), got_source_id);
        let (got_name, got_name_id) = get_name(&sub_fwd).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(None, got_name_id);
        let ret = clear_agp_header(&mut sub_fwd);
        assert_eq!(Ok(()), ret);
        assert_eq!(
            create_agp_header(&source, &name, None, None, None, None, None),
            get_agp_header(&sub_fwd)
        );

        let unsub_from = create_unsubscription_from(&name, Some(2), 50);

        assert_eq!(header_from, get_agp_header(&unsub_from));
        assert_eq!(Some(50), get_recv_from(&unsub_from).unwrap());
        assert_eq!(None, get_forward_to(&unsub_from).unwrap());
        assert_eq!(None, get_incoming_connection(&sub_from).unwrap());
        let (got_source, got_source_id) = get_source(&unsub_from).unwrap();
        assert_eq!(*Agent::default().agent_type(), got_source);
        assert_eq!(Agent::default().agent_id_option(), got_source_id);
        let (got_name, got_name_id) = get_name(&unsub_from).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(Some(2), got_name_id);

        let mut unsub_fwd = create_unsubscription_to_forward(&source, &name, None, 30);

        assert_eq!(header_fwd, get_agp_header(&unsub_fwd));
        assert_eq!(None, get_recv_from(&unsub_fwd).unwrap());
        assert_eq!(Some(30), get_forward_to(&unsub_fwd).unwrap());
        assert_eq!(None, get_incoming_connection(&unsub_fwd).unwrap());
        let (got_source, got_source_id) = get_source(&unsub_fwd).unwrap();
        assert_eq!(*source.agent_type(), got_source);
        assert_eq!(Some(1), got_source_id);
        let (got_name, got_name_id) = get_name(&unsub_fwd).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(None, got_name_id);
        let ret = clear_agp_header(&mut unsub_fwd);
        assert_eq!(Ok(()), ret);
        assert_eq!(
            create_agp_header(&source, &name, None, None, None, None, None),
            get_agp_header(&unsub_fwd)
        );

        let mut p = create_publication(
            header,
            create_default_service_header(),
            HashMap::new(),
            10,
            "str",
            "this is the content of the message".into(),
        );
        assert_eq!(header, get_agp_header(&p));
        assert_eq!(None, get_recv_from(&p).unwrap());
        assert_eq!(None, get_forward_to(&p).unwrap());
        assert_eq!(None, get_incoming_connection(&p).unwrap());
        let (got_source, got_source_id) = get_source(&sub).unwrap();
        assert_eq!(*source.agent_type(), got_source);
        assert_eq!(Some(1), got_source_id);
        let (got_name, got_name_id) = get_name(&sub).unwrap();
        assert_eq!(name, got_name);
        assert_eq!(Some(2), got_name_id);

        let ret = set_incoming_connection(&mut p, Some(500));
        assert_eq!(Ok(()), ret);
        assert_eq!(get_incoming_connection(&p).unwrap(), Some(500));
        let ret = clear_agp_header(&mut p);
        assert_eq!(Ok(()), ret);
        assert_eq!(
            create_agp_header(&source, &name, Some(2), None, None, Some(500), None),
            get_agp_header(&p)
        );
        let msg = match &p.message_type {
            Some(ProtoPublishType(msg)) => msg,
            // this should never happen
            _ => panic!("wrong message type"),
        };

        assert_eq!(get_fanout(&msg), 10);
        assert_eq!(
            get_payload(&msg),
            "this is the content of the message".as_bytes()
        );
    }
}
