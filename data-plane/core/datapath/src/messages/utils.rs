// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Display;
use std::{collections::HashMap, time::Duration};

use super::encoder::Name;
use crate::api::proto::dataplane::v1::{
    GroupClosePayload, GroupNackPayload, Participant, ParticipantSettings, PingPayload,
};
use crate::api::{
    Content, LinkNegotiationPayload, MessageType, ProtoLink, ProtoLinkMessageType, ProtoLinkType,
    ProtoMessage, ProtoName, ProtoPublish, ProtoPublishType, ProtoSessionType, ProtoSubscribe,
    ProtoSubscribeType, ProtoSubscriptionAck, ProtoSubscriptionAckType, ProtoUnsubscribe,
    ProtoUnsubscribeType, SessionHeader, SlimHeader,
    proto::dataplane::v1::{
        ApplicationPayload, CommandPayload, DiscoveryReplyPayload, DiscoveryRequestPayload,
        EncodedName, GroupAckPayload, GroupAddPayload, GroupProposalPayload, GroupRemovePayload,
        GroupWelcomePayload, JoinReplyPayload, JoinRequestPayload, LeaveReplyPayload,
        LeaveRequestPayload, MlsPayload, SessionMessageType, StringName, TimerSettings,
        command_payload::CommandPayloadType, content::ContentType,
    },
};

use slim_version::version;
use thiserror::Error;

/// IS_MODERATOR is used in the Join request metadata to indicate whether a participant is the moderator/initiator of a session.
/// The value is set to `TRUE_VAL` when the participant is a moderator.
pub const IS_MODERATOR: &str = "IS_MODERATOR";

/// DELETE_GROUP indicates that the entire group should be deleted.
/// When added to a leave request message, it signals to the moderator that the group needs to be closed.
/// The value is set to `TRUE_VAL` to trigger group deletion.
pub const DELETE_GROUP: &str = "DELETE_GROUP";

/// PUBLISH_TO indicates that a message should bypass normal sequencing and be delivered directly to the specified endpoint.
/// This is used in group sessions when the application API `publish_to` is used instead of `publish`.
/// The value is set to `TRUE_VAL` for direct delivery without buffering.
pub const PUBLISH_TO: &str = "PUBLISH_TO";

/// DISCONNECTION_DETECTED indicates that a participant disconnection was detected (not a graceful leave).
/// This is used in the leave request message and internally by the moderator when
/// a disconnection is detected due to missing ping replies from the participant.
/// The value is set to `TRUE_VAL` when disconnection is detected.
pub const DISCONNECTION_DETECTED: &str = "DISCONNECTION_DETECTED";

/// LEAVING_SESSION indicates that a participant is gracefully leaving the session.
/// This is used in the leave request message sent by a participant closing the session to the moderator.
/// The value is set to `TRUE_VAL` for graceful departure.
pub const LEAVING_SESSION: &str = "LEAVING_SESSION";

/// Standard string value representing a boolean "true" in message metadata.
pub const TRUE_VAL: &str = "TRUE";

/// Standard string value representing a boolean "false" in message metadata.
pub const FALSE_VAL: &str = "FALSE";

/// Maximum message ID for normal sequenced messages.
/// Messages with IDs in the range [0, MAX_PUBLISH_ID] follow normal sequencing.
/// Messages with IDs > MAX_PUBLISH_ID (used for `PUBLISH_TO` messages) bypass sequencing.
/// Value: Half of u32::MAX to allow a separate ID space for out-of-band messages.
pub const MAX_PUBLISH_ID: u32 = u32::MAX / 2;

#[derive(Error, Debug, PartialEq)]
pub enum MessageError {
    #[error("SLIM header not found")]
    SlimHeaderNotFound,
    #[error("source not found")]
    SourceNotFound,
    #[error("destination not found")]
    DestinationNotFound,
    #[error("session header not found")]
    SessionHeaderNotFound,
    #[error("message type not found")]
    MessageTypeNotFound,
    #[error("incoming connection not found")]
    IncomingConnectionNotFound,
    #[error("content type is not set")]
    ContentTypeNotSet,
    #[error("content is not an application payload")]
    NotApplicationPayload,
    #[error("content is not a command payload")]
    NotCommandPayload,
    #[error("link type is not set")]
    LinkTypeNotSet,
    #[error("invalid command payload type: expected {expected}, got {got}")]
    InvalidCommandPayloadType {
        expected: Box<String>,
        got: Box<String>,
    },

    // Builder errors
    #[error("builder error: source is required")]
    BuilderErrorSourceRequired,
    #[error("builder error: destination is required")]
    BuilderErrorDestinationRequired,
    #[error("participant name not found")]
    ParticipantNameNotFound,
    #[error("participant settings not found")]
    ParticipantSettingsNotFound,
}

/// ProtoName from Name
impl From<&Name> for ProtoName {
    fn from(name: &Name) -> Self {
        Self {
            name: Some(EncodedName {
                component_0: name.components()[0],
                component_1: name.components()[1],
                component_2: name.components()[2],
                component_3: name.components()[3],
            }),
            str_name: Some(StringName {
                str_component_0: name.components_strings()[0].clone(),
                str_component_1: name.components_strings()[1].clone(),
                str_component_2: name.components_strings()[2].clone(),
            }),
        }
    }
}

impl ParticipantSettings {
    /// Creates bidirectional participant settings (both sends and receives data).
    /// This is the most common configuration for participants in a session.
    pub fn bidirectional() -> Self {
        Self {
            sends_data: true,
            receives_data: true,
        }
    }

    /// Creates send-only participant settings.
    pub fn send_only() -> Self {
        Self {
            sends_data: true,
            receives_data: false,
        }
    }

    /// Creates receive-only participant settings.
    pub fn receive_only() -> Self {
        Self {
            sends_data: false,
            receives_data: true,
        }
    }

    /// Returns whether this participant produces data messages.
    pub fn is_sender(&self) -> bool {
        self.sends_data
    }

    /// Returns whether this participant consumes data messages.
    pub fn is_receiver(&self) -> bool {
        self.receives_data
    }
}

impl Participant {
    pub fn new(name: Name, settings: ParticipantSettings) -> Self {
        Self {
            name: Some(ProtoName::from(&name)),
            settings: Some(settings),
        }
    }

    pub fn get_name(&self) -> Result<Name, MessageError> {
        match &self.name {
            Some(name) => Ok(Name::from(name)),
            None => Err(MessageError::ParticipantNameNotFound),
        }
    }

    pub fn get_settings(&self) -> Result<&ParticipantSettings, MessageError> {
        match &self.settings {
            Some(settings) => Ok(settings),
            None => Err(MessageError::ParticipantSettingsNotFound),
        }
    }
}

/// Print message type
impl Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageType::Publish(_) => write!(f, "publish"),
            MessageType::Subscribe(_) => write!(f, "subscribe"),
            MessageType::Unsubscribe(_) => write!(f, "unsubscribe"),
            MessageType::Link(_) => write!(f, "link"),
            MessageType::SubscriptionAck(_) => write!(f, "subscription_ack"),
        }
    }
}

/// Struct grouping the SLIMHeaeder flags for convenience
#[derive(Debug, Clone)]
pub struct SlimHeaderFlags {
    pub fanout: u32,
    pub recv_from: Option<u64>,
    pub forward_to: Option<u64>,
    pub incoming_conn: Option<u64>,
    pub error: Option<bool>,
}

impl Default for SlimHeaderFlags {
    fn default() -> Self {
        Self {
            fanout: 1,
            recv_from: None,
            forward_to: None,
            incoming_conn: None,
            error: None,
        }
    }
}

impl Display for SlimHeaderFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fanout: {}, recv_from: {:?}, forward_to: {:?}, incoming_conn: {:?}, error: {:?}",
            self.fanout, self.recv_from, self.forward_to, self.incoming_conn, self.error
        )
    }
}

impl SlimHeaderFlags {
    pub fn new(
        fanout: u32,
        recv_from: Option<u64>,
        forward_to: Option<u64>,
        incoming_conn: Option<u64>,
        error: Option<bool>,
    ) -> Self {
        Self {
            fanout,
            recv_from,
            forward_to,
            incoming_conn,
            error,
        }
    }

    pub fn with_fanout(self, fanout: u32) -> Self {
        Self { fanout, ..self }
    }

    pub fn with_recv_from(self, recv_from: u64) -> Self {
        Self {
            recv_from: Some(recv_from),
            ..self
        }
    }

    pub fn with_forward_to(self, forward_to: u64) -> Self {
        Self {
            forward_to: Some(forward_to),
            ..self
        }
    }

    pub fn with_incoming_conn(self, incoming_conn: u64) -> Self {
        Self {
            incoming_conn: Some(incoming_conn),
            ..self
        }
    }

    pub fn with_error(self, error: bool) -> Self {
        Self {
            error: Some(error),
            ..self
        }
    }
}

/// SLIM Header
/// This header is used to identify the source and destination of the message
/// and to manage the connections used to send and receive the message
impl SlimHeader {
    pub fn new(
        source: &Name,
        destination: &Name,
        identity: &str,
        flags: Option<SlimHeaderFlags>,
    ) -> Self {
        let flags = flags.unwrap_or_default();

        Self {
            source: Some(ProtoName::from(source)),
            destination: Some(ProtoName::from(destination)),
            identity: identity.to_string(),
            fanout: flags.fanout,
            version: version().to_string(),
            recv_from: flags.recv_from,
            forward_to: flags.forward_to,
            incoming_conn: flags.incoming_conn,
            error: flags.error,
        }
    }

    pub fn clear_flags(&mut self) {
        self.recv_from = None;
        self.forward_to = None;
    }

    pub fn get_fanout(&self) -> u32 {
        self.fanout
    }

    pub fn get_recv_from(&self) -> Option<u64> {
        self.recv_from
    }

    pub fn get_forward_to(&self) -> Option<u64> {
        self.forward_to
    }

    pub fn get_incoming_conn(&self) -> Option<u64> {
        self.incoming_conn
    }

    pub fn get_error(&self) -> Option<bool> {
        self.error
    }

    pub fn get_source(&self) -> Name {
        match &self.source {
            Some(source) => Name::from(source),
            None => panic!("source not found"),
        }
    }

    pub fn get_dst(&self) -> Name {
        match &self.destination {
            Some(destination) => Name::from(destination),
            None => panic!("destination not found"),
        }
    }

    pub fn get_identity(&self) -> String {
        self.identity.clone()
    }

    pub fn get_version(&self) -> String {
        self.version.clone()
    }

    pub fn set_source(&mut self, source: &Name) {
        self.source = Some(ProtoName::from(source));
    }

    pub fn set_destination(&mut self, dst: &Name) {
        self.destination = Some(ProtoName::from(dst));
    }

    pub fn set_identity(&mut self, identity: String) {
        self.identity = identity;
    }

    pub fn set_fanout(&mut self, fanout: u32) {
        self.fanout = fanout;
    }

    pub fn set_recv_from(&mut self, recv_from: Option<u64>) {
        self.recv_from = recv_from;
    }

    pub fn set_forward_to(&mut self, forward_to: Option<u64>) {
        self.forward_to = forward_to;
    }

    pub fn set_error(&mut self, error: Option<bool>) {
        self.error = error;
    }

    pub fn set_incoming_conn(&mut self, incoming_conn: Option<u64>) {
        self.incoming_conn = incoming_conn;
    }

    pub fn set_error_flag(&mut self, error: Option<bool>) {
        self.error = error;
    }

    // returns (incoming, recv_from, forward_to) for subscription processing
    pub(crate) fn get_connections(&self) -> (u64, Option<u64>, Option<u64>) {
        // when calling this function, incoming connection is set
        let incoming = self
            .get_incoming_conn()
            .expect("incoming connection not found");

        (incoming, self.get_recv_from(), self.get_forward_to())
    }
}

/// Session Header
/// This header is used to identify the session and the message
/// and to manage session state
impl SessionHeader {
    pub fn new(
        session_type: i32,
        session_message_type: i32,
        session_id: u32,
        message_id: u32,
    ) -> Self {
        Self {
            session_type,
            session_message_type,
            session_id,
            message_id,
        }
    }

    pub fn get_session_id(&self) -> u32 {
        self.session_id
    }

    pub fn get_message_id(&self) -> u32 {
        self.message_id
    }

    pub fn set_session_id(&mut self, session_id: u32) {
        self.session_id = session_id;
    }

    pub fn set_message_id(&mut self, message_id: u32) {
        self.message_id = message_id;
    }

    pub fn clear(&mut self) {
        self.session_id = 0;
        self.message_id = 0;
    }
}

/// SessionMessageType
/// Helper methods for session message types
impl SessionMessageType {
    /// Check if a message type is a command message (not application data)
    pub fn is_command_message(&self) -> bool {
        matches!(
            self,
            SessionMessageType::DiscoveryRequest
                | SessionMessageType::DiscoveryReply
                | SessionMessageType::JoinRequest
                | SessionMessageType::JoinReply
                | SessionMessageType::LeaveRequest
                | SessionMessageType::LeaveReply
                | SessionMessageType::GroupAdd
                | SessionMessageType::GroupRemove
                | SessionMessageType::GroupWelcome
                | SessionMessageType::GroupClose
                | SessionMessageType::GroupProposal
                | SessionMessageType::GroupAck
                | SessionMessageType::GroupNack
                | SessionMessageType::Ping
        )
    }
}

/// ProtoSubscribe
/// This message is used to subscribe to a topic
impl ProtoSubscribe {
    fn new(
        source: &Name,
        dst: &Name,
        identity: Option<&str>,
        flags: Option<SlimHeaderFlags>,
    ) -> Self {
        let id = identity.unwrap_or("");
        let header = Some(SlimHeader::new(source, dst, id, flags));

        ProtoSubscribe {
            header,
            subscription_id: 0,
        }
    }
}

/// From ProtoMessage to ProtoSubscribe
impl From<ProtoMessage> for ProtoSubscribe {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoSubscribeType(s)) => s,
            _ => panic!("message type is not subscribe"),
        }
    }
}

/// ProtoUnsubscribe
/// This message is used to unsubscribe from a topic
impl ProtoUnsubscribe {
    fn new(
        source: &Name,
        dst: &Name,
        identity: Option<&str>,
        flags: Option<SlimHeaderFlags>,
    ) -> Self {
        let id = identity.unwrap_or("");
        let header = Some(SlimHeader::new(source, dst, id, flags));

        ProtoUnsubscribe {
            header,
            subscription_id: 0,
        }
    }
}

/// From ProtoMessage to ProtoUnsubscribe
impl From<ProtoMessage> for ProtoUnsubscribe {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoUnsubscribeType(u)) => u,
            _ => panic!("message type is not unsubscribe"),
        }
    }
}

/// ProtoPublish
/// This message is used to publish a message, either to a shared channel or to a specific application
impl ProtoPublish {
    fn with_header(
        header: Option<SlimHeader>,
        session: Option<SessionHeader>,
        payload: Option<Content>,
    ) -> Self {
        ProtoPublish {
            header,
            session,
            msg: payload,
        }
    }

    pub fn get_slim_header(&self) -> &SlimHeader {
        self.header.as_ref().unwrap()
    }

    pub fn get_session_header(&self) -> &SessionHeader {
        self.session.as_ref().unwrap()
    }

    pub fn get_slim_header_as_mut(&mut self) -> &mut SlimHeader {
        self.header.as_mut().unwrap()
    }

    pub fn get_session_header_as_mut(&mut self) -> &mut SessionHeader {
        self.session.as_mut().unwrap()
    }

    pub fn get_payload(&self) -> &Content {
        self.msg.as_ref().unwrap()
    }

    pub fn set_payload(&mut self, payload: Content) {
        self.msg = Some(payload);
    }

    pub fn is_command(&self) -> bool {
        match &self.get_payload().content_type.as_ref().unwrap() {
            ContentType::AppPayload(_) => false,
            ContentType::CommandPayload(_) => true,
        }
    }

    pub fn get_application_payload(&self) -> &ApplicationPayload {
        match self.get_payload().content_type.as_ref().unwrap() {
            ContentType::AppPayload(application_payload) => application_payload,
            ContentType::CommandPayload(_) => panic!("the payload is not an application payload"),
        }
    }

    pub fn get_command_payload(&self) -> &CommandPayload {
        match &self.get_payload().content_type.as_ref().unwrap() {
            ContentType::AppPayload(_) => panic!("the payaoad is not a command payload"),
            ContentType::CommandPayload(command_payload) => command_payload,
        }
    }
}

/// From ProtoMessage to ProtoPublish
impl From<ProtoMessage> for ProtoPublish {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoPublishType(p)) => p,
            _ => panic!("message type is not publish"),
        }
    }
}

/// ProtoMessage
/// This represents a generic message that can be sent over the network
// Macro to generate payload extraction methods for ProtoMessage
macro_rules! impl_payload_extractors {
    ($($method_name:ident => $getter_method:ident($payload_type:ty)),* $(,)?) => {
        $(
            /// Extracts a specific command payload from the message.
            pub fn $method_name(&self) -> Result<&$payload_type, MessageError> {
                self.extract_command_payload()?.$getter_method()
            }
        )*
    };
}

impl ProtoMessage {
    fn new(metadata: HashMap<String, String>, message_type: MessageType) -> Self {
        ProtoMessage {
            metadata,
            message_type: Some(message_type),
        }
    }

    fn validate_link(link: &ProtoLink) -> Result<(), MessageError> {
        if link.link_type.is_none() {
            return Err(MessageError::LinkTypeNotSet);
        }
        Ok(())
    }

    fn validate_routed_header(slim_header: &SlimHeader) -> Result<(), MessageError> {
        if slim_header.source.is_none() {
            return Err(MessageError::SourceNotFound);
        }
        if slim_header.destination.is_none() {
            return Err(MessageError::DestinationNotFound);
        }
        Ok(())
    }

    fn validate_publish(p: &ProtoPublish) -> Result<(), MessageError> {
        let hdr = p.header.as_ref().ok_or(MessageError::SlimHeaderNotFound)?;
        Self::validate_routed_header(hdr)?;
        if p.session.is_none() {
            return Err(MessageError::SessionHeaderNotFound);
        }
        Ok(())
    }

    fn validate_subscribe(s: &ProtoSubscribe) -> Result<(), MessageError> {
        let hdr = s.header.as_ref().ok_or(MessageError::SlimHeaderNotFound)?;
        Self::validate_routed_header(hdr)
    }

    fn validate_unsubscribe(u: &ProtoUnsubscribe) -> Result<(), MessageError> {
        let hdr = u.header.as_ref().ok_or(MessageError::SlimHeaderNotFound)?;
        Self::validate_routed_header(hdr)
    }

    // validate message
    pub fn validate(&self) -> Result<(), MessageError> {
        match &self.message_type {
            None => Err(MessageError::MessageTypeNotFound),
            Some(ProtoLinkMessageType(link)) => Self::validate_link(link),
            Some(ProtoPublishType(p)) => Self::validate_publish(p),
            Some(ProtoSubscribeType(s)) => Self::validate_subscribe(s),
            Some(ProtoUnsubscribeType(u)) => Self::validate_unsubscribe(u),
            Some(ProtoSubscriptionAckType(_)) => Ok(()),
        }
    }

    // add metadata key in the map assigning the value val
    // if the key exists the value is replaced by val
    pub fn insert_metadata(&mut self, key: String, val: String) {
        self.metadata.insert(key, val);
    }

    // remove metadata key from the map
    pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
        self.metadata.remove(key)
    }

    pub fn contains_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key)
    }

    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }

    pub fn get_metadata_map(&self) -> HashMap<String, String> {
        self.metadata.clone()
    }

    pub fn set_metadata_map(&mut self, map: HashMap<String, String>) {
        for (k, v) in map.iter() {
            self.insert_metadata(k.to_string(), v.to_string());
        }
    }

    pub fn get_slim_header(&self) -> &SlimHeader {
        match &self.message_type {
            Some(ProtoPublishType(publish)) => publish.header.as_ref().unwrap(),
            Some(ProtoSubscribeType(sub)) => sub.header.as_ref().unwrap(),
            Some(ProtoUnsubscribeType(unsub)) => unsub.header.as_ref().unwrap(),
            Some(ProtoLinkMessageType(_)) | Some(ProtoSubscriptionAckType(_)) | None => {
                panic!("SLIM header not found")
            }
        }
    }

    pub fn get_slim_header_mut(&mut self) -> &mut SlimHeader {
        match &mut self.message_type {
            Some(ProtoPublishType(publish)) => publish.header.as_mut().unwrap(),
            Some(ProtoSubscribeType(sub)) => sub.header.as_mut().unwrap(),
            Some(ProtoUnsubscribeType(unsub)) => unsub.header.as_mut().unwrap(),
            Some(ProtoLinkMessageType(_)) | Some(ProtoSubscriptionAckType(_)) | None => {
                panic!("SLIM header not found")
            }
        }
    }

    pub fn try_get_slim_header(&self) -> Option<&SlimHeader> {
        match &self.message_type {
            Some(ProtoPublishType(publish)) => publish.header.as_ref(),
            Some(ProtoSubscribeType(sub)) => sub.header.as_ref(),
            Some(ProtoUnsubscribeType(unsub)) => unsub.header.as_ref(),
            Some(ProtoLinkMessageType(_)) | Some(ProtoSubscriptionAckType(_)) | None => None,
        }
    }

    pub fn get_session_header(&self) -> &SessionHeader {
        match &self.message_type {
            Some(ProtoPublishType(publish)) => publish.session.as_ref().unwrap(),
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => panic!("session header not found"),
        }
    }

    pub fn get_session_header_mut(&mut self) -> &mut SessionHeader {
        match &mut self.message_type {
            Some(ProtoPublishType(publish)) => publish.session.as_mut().unwrap(),
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => panic!("session header not found"),
        }
    }

    pub fn try_get_session_header(&self) -> Option<&SessionHeader> {
        match &self.message_type {
            Some(ProtoPublishType(publish)) => publish.session.as_ref(),
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => None,
        }
    }

    pub fn try_get_session_header_mut(&mut self) -> Option<&mut SessionHeader> {
        match &mut self.message_type {
            Some(ProtoPublishType(publish)) => publish.session.as_mut(),
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => None,
        }
    }

    pub fn get_id(&self) -> u32 {
        self.get_session_header().get_message_id()
    }

    pub fn get_source(&self) -> Name {
        self.get_slim_header().get_source()
    }

    pub fn get_dst(&self) -> Name {
        self.get_slim_header().get_dst()
    }

    pub fn get_identity(&self) -> String {
        self.get_slim_header().get_identity()
    }

    pub fn get_fanout(&self) -> u32 {
        self.get_slim_header().get_fanout()
    }

    pub fn get_recv_from(&self) -> Option<u64> {
        self.get_slim_header().get_recv_from()
    }

    pub fn get_forward_to(&self) -> Option<u64> {
        self.get_slim_header().get_forward_to()
    }

    pub fn get_error(&self) -> Option<bool> {
        self.get_slim_header().get_error()
    }

    pub fn get_incoming_conn(&self) -> u64 {
        self.get_slim_header().get_incoming_conn().unwrap()
    }

    pub fn try_get_incoming_conn(&self) -> Option<u64> {
        self.get_slim_header().get_incoming_conn()
    }

    pub fn get_type(&self) -> &MessageType {
        match &self.message_type {
            Some(t) => t,
            None => panic!("message type not found"),
        }
    }

    pub fn get_payload(&self) -> Option<&Content> {
        match &self.message_type {
            Some(ProtoPublishType(p)) => p.msg.as_ref(),
            Some(ProtoSubscribeType(_)) => panic!("payload not found"),
            Some(ProtoUnsubscribeType(_)) => panic!("payload not found"),
            Some(ProtoLinkMessageType(_)) => panic!("payload not found"),
            Some(ProtoSubscriptionAckType(_)) => panic!("payload not found"),
            None => panic!("payload not found"),
        }
    }

    pub fn set_payload(&mut self, payload: Content) {
        match &mut self.message_type {
            Some(ProtoPublishType(p)) => p.set_payload(payload),
            Some(ProtoSubscribeType(_)) => panic!("no payload allowed"),
            Some(ProtoUnsubscribeType(_)) => panic!("no payload allowed"),
            Some(ProtoLinkMessageType(_)) => panic!("no payload allowed"),
            Some(ProtoSubscriptionAckType(_)) => panic!("no payload allowed"),
            None => panic!("no payload allowed"),
        }
    }

    pub fn get_session_message_type(&self) -> SessionMessageType {
        self.get_session_header()
            .session_message_type
            .try_into()
            .unwrap_or_default()
    }

    pub fn clear_slim_header(&mut self) {
        if self.is_link() || self.is_subscription_ack() {
            return;
        }
        self.get_slim_header_mut().clear_flags();
    }

    pub fn set_recv_from(&mut self, recv_from: Option<u64>) {
        self.get_slim_header_mut().set_recv_from(recv_from);
    }

    pub fn set_forward_to(&mut self, forward_to: Option<u64>) {
        self.get_slim_header_mut().set_forward_to(forward_to);
    }

    pub fn set_error(&mut self, error: Option<bool>) {
        self.get_slim_header_mut().set_error(error);
    }

    pub fn set_fanout(&mut self, fanout: u32) {
        self.get_slim_header_mut().set_fanout(fanout);
    }

    pub fn set_incoming_conn(&mut self, incoming_conn: Option<u64>) {
        self.get_slim_header_mut().set_incoming_conn(incoming_conn);
    }

    pub fn set_error_flag(&mut self, error: Option<bool>) {
        self.get_slim_header_mut().set_error_flag(error);
    }

    pub fn set_session_message_type(&mut self, message_type: SessionMessageType) {
        self.get_session_header_mut()
            .set_session_message_type(message_type);
    }

    pub fn set_session_type(&mut self, session_type: ProtoSessionType) {
        self.get_session_header_mut().set_session_type(session_type);
    }

    pub fn get_session_type(&self) -> ProtoSessionType {
        self.get_session_header().session_type()
    }

    pub fn set_message_id(&mut self, message_id: u32) {
        self.get_session_header_mut().set_message_id(message_id);
    }

    pub fn is_publish(&self) -> bool {
        matches!(self.get_type(), MessageType::Publish(_))
    }

    pub fn is_subscribe(&self) -> bool {
        matches!(self.get_type(), MessageType::Subscribe(_))
    }

    pub fn is_unsubscribe(&self) -> bool {
        matches!(self.get_type(), MessageType::Unsubscribe(_))
    }

    pub fn is_link(&self) -> bool {
        matches!(self.get_type(), MessageType::Link(_))
    }

    pub fn is_subscription_ack(&self) -> bool {
        matches!(self.get_type(), MessageType::SubscriptionAck(_))
    }

    pub fn get_subscription_ack(&self) -> &ProtoSubscriptionAck {
        match &self.message_type {
            Some(ProtoSubscriptionAckType(ack)) => ack,
            _ => panic!("message type is not subscription_ack"),
        }
    }

    /// Returns the subscription_id from a Subscribe/Unsubscribe message, or None if absent/zero.
    pub fn get_subscription_id(&self) -> Option<u64> {
        match &self.message_type {
            Some(ProtoSubscribeType(s)) if s.subscription_id != 0 => Some(s.subscription_id),
            Some(ProtoUnsubscribeType(u)) if u.subscription_id != 0 => Some(u.subscription_id),
            _ => None,
        }
    }

    /// Takes and clears the subscription_id from a Subscribe/Unsubscribe message.
    /// Returns None if the field is absent or zero.
    pub fn take_subscription_id(&mut self) -> Option<u64> {
        match &mut self.message_type {
            Some(ProtoSubscribeType(s)) if s.subscription_id != 0 => {
                Some(std::mem::take(&mut s.subscription_id))
            }
            Some(ProtoUnsubscribeType(u)) if u.subscription_id != 0 => {
                Some(std::mem::take(&mut u.subscription_id))
            }
            _ => None,
        }
    }

    /// Sets the subscription_id on a Subscribe/Unsubscribe message (no-op for other types).
    pub fn set_subscription_id(&mut self, subscription_id: u64) {
        match &mut self.message_type {
            Some(ProtoSubscribeType(s)) => s.subscription_id = subscription_id,
            Some(ProtoUnsubscribeType(u)) => u.subscription_id = subscription_id,
            _ => {}
        }
    }

    /// Extracts the command payload from the message.
    ///
    /// # Errors
    /// Returns `MessageError` if the payload is missing or cannot be converted.
    pub fn extract_command_payload(&self) -> Result<&CommandPayload, MessageError> {
        self.get_payload()
            .ok_or(MessageError::ContentTypeNotSet)?
            .as_command_payload()
    }

    // Generate all payload extraction methods
    impl_payload_extractors! {
        extract_discovery_request => as_discovery_request_payload(DiscoveryRequestPayload),
        extract_discovery_reply => as_discovery_reply_payload(DiscoveryReplyPayload),
        extract_join_request => as_join_request_payload(JoinRequestPayload),
        extract_join_reply => as_join_reply_payload(JoinReplyPayload),
        extract_leave_request => as_leave_request_payload(LeaveRequestPayload),
        extract_leave_reply => as_leave_reply_payload(LeaveReplyPayload),
        extract_group_add => as_group_add_payload(GroupAddPayload),
        extract_group_remove => as_group_remove_payload(GroupRemovePayload),
        extract_group_welcome => as_welcome_payload(GroupWelcomePayload),
        extract_group_close => as_group_close_payload(GroupClosePayload),
        extract_group_proposal => as_group_proposal_payload(GroupProposalPayload),
        extract_group_ack => as_group_ack_payload(GroupAckPayload),
        extract_group_nack => as_group_nack_payload(GroupNackPayload),
        extract_ping => as_ping_payload(PingPayload),
    }
}

impl Content {
    pub fn as_application_payload(&self) -> Result<&ApplicationPayload, MessageError> {
        match &self.content_type {
            Some(ContentType::AppPayload(app_payload)) => Ok(app_payload),
            Some(ContentType::CommandPayload(_)) => Err(MessageError::NotApplicationPayload),
            None => Err(MessageError::ContentTypeNotSet),
        }
    }

    pub fn as_command_payload(&self) -> Result<&CommandPayload, MessageError> {
        match &self.content_type {
            Some(ContentType::AppPayload(_)) => Err(MessageError::NotCommandPayload),
            Some(ContentType::CommandPayload(comm_payload)) => Ok(comm_payload),
            None => Err(MessageError::ContentTypeNotSet),
        }
    }
}

impl ApplicationPayload {
    pub fn new(payload_type: &str, blob: Vec<u8>) -> Self {
        Self {
            payload_type: payload_type.to_string(),
            blob,
        }
    }

    pub fn as_content(&self) -> Content {
        Content {
            content_type: Some(ContentType::AppPayload(self.clone())),
        }
    }
}

// Macro to generate getter methods for all CommandPayloadType variants
macro_rules! impl_command_payload_getters {
    ($(
        $method_name:ident => $variant:ident($payload_type:ty)
    ),* $(,)?) => {
        $(
            pub fn $method_name(&self) -> Result<&$payload_type, MessageError> {
                match &self.command_payload_type {
                    Some(CommandPayloadType::$variant(payload)) => Ok(payload),
                    Some(other) => Err(MessageError::InvalidCommandPayloadType {
                        expected: Box::new(stringify!($variant).to_string()),
                        got: Box::new(format!("{:?}", other)),
                    }),
                    None => Err(MessageError::InvalidCommandPayloadType {
                        expected: Box::new(stringify!($variant).to_string()),
                        got: Box::new("None".to_string()),
                    }),
                }
            }
        )*
    };
}

impl CommandPayload {
    pub fn as_content(self) -> Content {
        Content {
            content_type: Some(ContentType::CommandPayload(self)),
        }
    }

    // Getter methods for all CommandPayloadType variants
    impl_command_payload_getters! {
        as_discovery_request_payload => DiscoveryRequest(DiscoveryRequestPayload),
        as_discovery_reply_payload => DiscoveryReply(DiscoveryReplyPayload),
        as_join_request_payload => JoinRequest(JoinRequestPayload),
        as_join_reply_payload => JoinReply(JoinReplyPayload),
        as_leave_request_payload => LeaveRequest(LeaveRequestPayload),
        as_leave_reply_payload => LeaveReply(LeaveReplyPayload),
        as_group_add_payload => GroupAdd(GroupAddPayload),
        as_group_remove_payload => GroupRemove(GroupRemovePayload),
        as_welcome_payload => GroupWelcome(GroupWelcomePayload),
        as_group_close_payload => GroupClose(GroupClosePayload),
        as_group_proposal_payload => GroupProposal(GroupProposalPayload),
        as_group_ack_payload => GroupAck(GroupAckPayload),
        as_group_nack_payload => GroupNack(GroupNackPayload),
        as_ping_payload => Ping(PingPayload),
    }
}

impl AsRef<ProtoPublish> for ProtoMessage {
    fn as_ref(&self) -> &ProtoPublish {
        match &self.message_type {
            Some(ProtoPublishType(p)) => p,
            _ => panic!("message type is not publish"),
        }
    }
}

/// Builder for creating CommandPayload instances with a fluent API
///
/// Provides methods for creating all types of command payloads.
///
/// # Examples
///
/// ## Discovery Request
/// ```
/// use slim_datapath::api::CommandPayload;
/// use slim_datapath::messages::Name;
///
/// let dest = Name::from_strings(["org", "namespace", "service"]);
/// let payload = CommandPayload::builder().discovery_request(Some(dest));
/// ```
///
/// ## Join Request with Timer Settings
/// ```
/// use slim_datapath::api::CommandPayload;
/// use slim_datapath::messages::Name;
/// use std::time::Duration;
///
/// let channel = Name::from_strings(["org", "namespace", "channel"]);
/// let payload = CommandPayload::builder().join_request(
///     true,  // enable_mls
///     Some(5),  // max_retries
///     Some(Duration::from_secs(10)),  // timeout
///     Some(channel),
/// );
/// ```
///
/// ## Group Operations
/// ```
/// use slim_datapath::api::CommandPayload;
/// use slim_datapath::messages::Name;
///
/// let participant = Name::from_strings(["org", "ns", "user1"]);
/// let participants = vec![
///     Name::from_strings(["org", "ns", "user2"]),
///     Name::from_strings(["org", "ns", "user3"]),
/// ];
///
/// // Add participant
/// let add_payload = CommandPayload::builder().group_add(
///     participant.clone(),
///     participants.clone(),
///     None,  // mls payload
/// );
/// ```
pub struct CommandPayloadBuilder;

impl CommandPayloadBuilder {
    /// Creates a new CommandPayloadBuilder
    pub fn new() -> Self {
        Self
    }

    /// Creates a discovery request payload
    pub fn discovery_request(self, destination: Option<Name>) -> CommandPayload {
        let proto_destination = destination.as_ref().map(ProtoName::from);
        let payload = DiscoveryRequestPayload {
            destination: proto_destination,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::DiscoveryRequest(payload)),
        }
    }

    /// Creates a discovery reply payload
    pub fn discovery_reply(self) -> CommandPayload {
        let payload = DiscoveryReplyPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::DiscoveryReply(payload)),
        }
    }

    /// Creates a join request payload
    pub fn join_request(
        self,
        enable_mls: bool,
        max_retries: Option<u32>,
        timer_duration: Option<Duration>,
        channel: Option<Name>,
    ) -> CommandPayload {
        let proto_channel = channel.as_ref().map(ProtoName::from);

        let timer_settings = if let Some(t) = timer_duration
            && let Some(m) = max_retries
        {
            Some(TimerSettings {
                timeout: t.as_millis() as u32,
                max_retries: m,
            })
        } else {
            None
        };

        let payload = JoinRequestPayload {
            enable_mls,
            timer_settings,
            channel: proto_channel,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::JoinRequest(payload)),
        }
    }

    /// Creates a join reply payload
    pub fn join_reply(
        self,
        key_package: Option<Vec<u8>>,
        participant: Participant,
    ) -> CommandPayload {
        let payload = JoinReplyPayload {
            key_package,
            participant: Some(participant),
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::JoinReply(payload)),
        }
    }

    /// Creates a leave request payload
    pub fn leave_request(self, destination: Option<Name>) -> CommandPayload {
        let proto_destination = destination.as_ref().map(ProtoName::from);
        let payload = LeaveRequestPayload {
            destination: proto_destination,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::LeaveRequest(payload)),
        }
    }

    /// Creates a leave reply payload
    pub fn leave_reply(self) -> CommandPayload {
        let payload = LeaveReplyPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::LeaveReply(payload)),
        }
    }

    /// Creates a group add payload
    pub fn group_add(
        self,
        new_participant: Participant,
        participants: Vec<Participant>,
        mls: Option<MlsPayload>,
    ) -> CommandPayload {
        let payload = GroupAddPayload {
            new_participant: Some(new_participant),
            participants,
            mls,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupAdd(payload)),
        }
    }

    /// Creates a group remove payload
    pub fn group_remove(
        self,
        removed_participant: Name,
        participants: Vec<Name>,
        mls: Option<MlsPayload>,
    ) -> CommandPayload {
        let proto_removed_participant = Some(ProtoName::from(&removed_participant));
        let proto_participants = participants.iter().map(ProtoName::from).collect();

        let payload = GroupRemovePayload {
            removed_participant: proto_removed_participant,
            participants: proto_participants,
            mls,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupRemove(payload)),
        }
    }

    /// Creates a group welcome payload
    pub fn group_welcome(
        self,
        participants: Vec<Participant>,
        mls: Option<MlsPayload>,
    ) -> CommandPayload {
        let payload = GroupWelcomePayload { participants, mls };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupWelcome(payload)),
        }
    }

    /// Creates a group close payload
    pub fn group_close(self, participants: Vec<Name>) -> CommandPayload {
        let proto_participants = participants.iter().map(ProtoName::from).collect();

        let payload = GroupClosePayload {
            participants: proto_participants,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupClose(payload)),
        }
    }

    /// Creates a group proposal payload
    pub fn group_proposal(self, source: Option<Name>, mls_proposal: Vec<u8>) -> CommandPayload {
        let proto_source = source.as_ref().map(ProtoName::from);
        let payload = GroupProposalPayload {
            source: proto_source,
            mls_proposal,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupProposal(payload)),
        }
    }

    /// Creates a group ack payload
    pub fn group_ack(self) -> CommandPayload {
        let payload = GroupAckPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupAck(payload)),
        }
    }

    /// Creates a group nack payload
    pub fn group_nack(self) -> CommandPayload {
        let payload = GroupNackPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupNack(payload)),
        }
    }

    /// Creates a ping payload
    pub fn ping(self) -> CommandPayload {
        let payload = PingPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::Ping(payload)),
        }
    }
}

impl Default for CommandPayloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPayload {
    /// Creates a new builder for CommandPayload
    pub fn builder() -> CommandPayloadBuilder {
        CommandPayloadBuilder::new()
    }
}

/// Builder for creating ProtoMessage instances with a fluent API
///
/// # Examples
///
/// ## Basic Publish Message
/// ```
/// use slim_datapath::api::{ProtoMessage, ProtoSessionType};
/// use slim_datapath::messages::Name;
///
/// let source = Name::from_strings(["org", "ns", "app"]).with_id(1);
/// let dest = Name::from_strings(["org", "ns", "service"]).with_id(2);
///
/// let msg = ProtoMessage::builder()
///     .source(source)
///     .destination(dest)
///     .session_type(ProtoSessionType::PointToPoint)
///     .session_id(123)
///     .application_payload("text", b"Hello".to_vec())
///     .build_publish()
///     .unwrap();
/// ```
///
/// ## Session Control Message
/// ```
/// use slim_datapath::api::{CommandPayload, ProtoMessage, ProtoSessionType, ProtoSessionMessageType};
/// use slim_datapath::messages::Name;
///
/// let source = Name::from_strings(["org", "ns", "app"]);
/// let dest = Name::from_strings(["org", "ns", "service"]);
///
/// let cmd = CommandPayload::builder().discovery_request(Some(dest.clone()));
///
/// let msg = ProtoMessage::builder()
///     .source(source)
///     .destination(dest)
///     .session_type(ProtoSessionType::PointToPoint)
///     .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
///     .session_id(42)
///     .command_payload(cmd)
///     .build_publish()
///     .unwrap();
/// ```
///
/// ## Multicast with Broadcast
/// ```
/// use slim_datapath::api::{ProtoMessage, ProtoSessionType};
/// use slim_datapath::messages::Name;
///
/// let source = Name::from_strings(["org", "ns", "app"]);
/// let dest = Name::from_strings(["org", "ns", "channel"]);
///
/// let msg = ProtoMessage::builder()
///     .source(source)
///     .destination(dest)
///     .session_type(ProtoSessionType::Multicast)
///     .fanout(256)
///     .application_payload("event", b"broadcast event".to_vec())
///     .metadata("priority", "high")
///     .build_publish()
///     .unwrap();
/// ```
///
/// ## Subscribe/Unsubscribe Messages
/// ```
/// use slim_datapath::api::ProtoMessage;
/// use slim_datapath::messages::Name;
///
/// let source = Name::from_strings(["org", "ns", "app"]);
/// let dest = Name::from_strings(["org", "ns", "topic"]);
///
/// // Subscribe
/// let sub_msg = ProtoMessage::builder()
///     .source(source.clone())
///     .destination(dest.clone())
///     .recv_from(100)
///     .build_subscribe()
///     .unwrap();
///
/// // Unsubscribe
/// let unsub_msg = ProtoMessage::builder()
///     .source(source)
///     .destination(dest)
///     .build_unsubscribe()
///     .unwrap();
/// ```
pub struct ProtoMessageBuilder {
    source: Option<Name>,
    destination: Option<Name>,
    identity: Option<String>,
    flags: Option<SlimHeaderFlags>,
    session_type: Option<ProtoSessionType>,
    session_message_type: Option<SessionMessageType>,
    session_id: Option<u32>,
    message_id: Option<u32>,
    payload: Option<Content>,
    metadata: HashMap<String, String>,
    subscription_id: Option<u64>,
}

impl ProtoMessageBuilder {
    /// Creates a new ProtoMessageBuilder
    pub fn new() -> Self {
        Self {
            source: None,
            destination: None,
            identity: None,
            flags: None,
            session_type: None,
            session_message_type: None,
            session_id: None,
            message_id: None,
            payload: None,
            metadata: HashMap::new(),
            subscription_id: None,
        }
    }

    /// Sets the source name
    pub fn source(mut self, source: Name) -> Self {
        self.source = Some(source);
        self
    }

    /// Sets the destination name
    pub fn destination(mut self, destination: Name) -> Self {
        self.destination = Some(destination);
        self
    }

    /// Sets the identity string
    pub fn identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// Sets the SLIM header flags
    pub fn flags(mut self, flags: SlimHeaderFlags) -> Self {
        self.flags = Some(flags);
        self
    }

    /// Sets the fanout value
    pub fn fanout(mut self, fanout: u32) -> Self {
        let flags = self.flags.take().unwrap_or_default();
        self.flags = Some(flags.with_fanout(fanout));
        self
    }

    /// Sets the recv_from connection
    pub fn recv_from(mut self, recv_from: u64) -> Self {
        let flags = self.flags.take().unwrap_or_default();
        self.flags = Some(flags.with_recv_from(recv_from));
        self
    }

    /// Sets the forward_to connection
    pub fn forward_to(mut self, forward_to: u64) -> Self {
        let flags = self.flags.take().unwrap_or_default();
        self.flags = Some(flags.with_forward_to(forward_to));
        self
    }

    /// Sets the incoming connection
    pub fn incoming_conn(mut self, incoming_conn: u64) -> Self {
        let flags = self.flags.take().unwrap_or_default();
        self.flags = Some(flags.with_incoming_conn(incoming_conn));
        self
    }

    /// Sets the error flag
    pub fn error(mut self, error: bool) -> Self {
        let flags = self.flags.take().unwrap_or_default();
        self.flags = Some(flags.with_error(error));
        self
    }

    /// Sets the session type
    pub fn session_type(mut self, session_type: ProtoSessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    /// Sets the session message type
    pub fn session_message_type(mut self, session_message_type: SessionMessageType) -> Self {
        self.session_message_type = Some(session_message_type);
        self
    }

    /// Sets the session ID
    pub fn session_id(mut self, session_id: u32) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Sets the message ID
    pub fn message_id(mut self, message_id: u32) -> Self {
        self.message_id = Some(message_id);
        self
    }

    /// Sets the message payload
    pub fn payload(mut self, payload: Content) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Sets an application payload
    pub fn application_payload(mut self, payload_type: &str, blob: Vec<u8>) -> Self {
        let app_payload = ApplicationPayload::new(payload_type, blob);
        self.payload = Some(app_payload.as_content());
        self
    }

    /// Sets a command payload
    pub fn command_payload(mut self, payload: CommandPayload) -> Self {
        self.payload = Some(payload.as_content());
        self
    }

    /// Sets a pre-built SlimHeader (for low-level use cases)
    ///
    /// This is a convenience method for cases where you already have a constructed SlimHeader.
    /// For most cases, prefer using the individual builder methods like `source()`, `destination()`, etc.
    pub fn with_slim_header(mut self, header: SlimHeader) -> Self {
        // Extract fields from the header
        if let Some(src) = &header.source {
            self.source = Some(Name::from(src));
        }
        if let Some(dst) = &header.destination {
            self.destination = Some(Name::from(dst));
        }
        if !header.identity.is_empty() {
            self.identity = Some(header.identity.clone());
        }

        // Extract flags
        let flags = SlimHeaderFlags {
            fanout: header.fanout,
            recv_from: header.recv_from,
            forward_to: header.forward_to,
            incoming_conn: header.incoming_conn,
            error: header.error,
        };
        self.flags = Some(flags);
        self
    }

    /// Sets a pre-built SessionHeader (for low-level use cases)
    ///
    /// This is a convenience method for cases where you already have a constructed SessionHeader.
    /// For most cases, prefer using the individual builder methods like `session_type()`, `session_message_type()`, etc.
    pub fn with_session_header(mut self, header: SessionHeader) -> Self {
        self.session_type = Some(
            ProtoSessionType::try_from(header.session_type)
                .unwrap_or(ProtoSessionType::PointToPoint),
        );
        self.session_message_type = Some(
            SessionMessageType::try_from(header.session_message_type)
                .unwrap_or(SessionMessageType::Msg),
        );
        self.session_id = Some(header.session_id);
        self.message_id = Some(header.message_id);
        self
    }

    /// Adds metadata to the message
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Adds multiple metadata entries
    pub fn metadata_map(mut self, map: HashMap<String, String>) -> Self {
        self.metadata.extend(map);
        self
    }

    /// Sets the subscription_id for subscribe/unsubscribe messages.
    pub fn subscription_id(mut self, id: u64) -> Self {
        self.subscription_id = Some(id);
        self
    }

    /// Builds a publish message
    pub fn build_publish(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let slim_header = Some(SlimHeader::new(
            &source,
            &destination,
            self.identity.as_deref().unwrap_or(""),
            self.flags,
        ));

        let session_header = if self.session_type.is_some() || self.session_message_type.is_some() {
            Some(SessionHeader::new(
                self.session_type
                    .unwrap_or(ProtoSessionType::PointToPoint)
                    .into(),
                self.session_message_type
                    .unwrap_or(SessionMessageType::Msg)
                    .into(),
                self.session_id.unwrap_or(0),
                self.message_id.unwrap_or_else(rand::random),
            ))
        } else {
            Some(SessionHeader::default())
        };

        let publish = ProtoPublish::with_header(slim_header, session_header, self.payload);
        let message = ProtoMessage::new(self.metadata, ProtoPublishType(publish));
        Ok(message)
    }

    /// Builds a subscribe message
    pub fn build_subscribe(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let mut subscribe =
            ProtoSubscribe::new(&source, &destination, self.identity.as_deref(), self.flags);
        subscribe.subscription_id = self.subscription_id.unwrap_or_default();

        Ok(ProtoMessage::new(
            self.metadata,
            ProtoSubscribeType(subscribe),
        ))
    }

    /// Builds an unsubscribe message
    pub fn build_unsubscribe(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let mut unsubscribe =
            ProtoUnsubscribe::new(&source, &destination, self.identity.as_deref(), self.flags);
        unsubscribe.subscription_id = self.subscription_id.unwrap_or_default();

        Ok(ProtoMessage::new(
            self.metadata,
            ProtoUnsubscribeType(unsubscribe),
        ))
    }

    /// Builds a subscription ack message.
    /// SubscriptionAck messages are delivered directly to the requesting connection
    /// and are never routed through the subscription table.
    pub fn build_subscription_ack(
        self,
        subscription_id: u64,
        success: bool,
        error: impl Into<String>,
    ) -> ProtoMessage {
        let ack = ProtoSubscriptionAck {
            subscription_id,
            success,
            error: error.into(),
        };
        ProtoMessage::new(self.metadata, ProtoSubscriptionAckType(ack))
    }

    /// Builds a link negotiation message.
    /// Link messages are link-local and never routed; they carry no SLIM header.
    pub fn build_link_negotiation(
        self,
        link_id: impl Into<String>,
        slim_version: impl Into<String>,
        is_reply: bool,
    ) -> ProtoMessage {
        let link = ProtoLink {
            link_type: Some(ProtoLinkType::LinkNegotiation(LinkNegotiationPayload {
                link_id: link_id.into(),
                slim_version: slim_version.into(),
                is_reply,
            })),
        };
        ProtoMessage::new(self.metadata, ProtoLinkMessageType(link))
    }
}

impl Default for ProtoMessageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtoMessage {
    /// Creates a new builder for ProtoMessage
    pub fn builder() -> ProtoMessageBuilder {
        ProtoMessageBuilder::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::{api::proto::dataplane::v1::SessionMessageType, messages::encoder::Name};

    use super::*;

    fn test_subscription_template(
        subscription: bool,
        source: Name,
        dst: Name,
        identity: Option<&str>,
        flags: Option<SlimHeaderFlags>,
    ) {
        let sub = {
            let mut builder = ProtoMessage::builder()
                .source(source.clone())
                .destination(dst.clone());

            if let Some(id) = identity {
                builder = builder.identity(id);
            }

            if let Some(f) = flags.clone() {
                builder = builder.flags(f);
            }

            if subscription {
                builder.build_subscribe().unwrap()
            } else {
                builder.build_unsubscribe().unwrap()
            }
        };

        let flags = if flags.is_none() {
            Some(SlimHeaderFlags::default())
        } else {
            flags
        };

        assert!(!sub.is_publish());
        assert_eq!(sub.is_subscribe(), subscription);
        assert_eq!(sub.is_unsubscribe(), !subscription);
        assert_eq!(flags.as_ref().unwrap().recv_from, sub.get_recv_from());
        assert_eq!(flags.as_ref().unwrap().forward_to, sub.get_forward_to());
        assert_eq!(None, sub.try_get_incoming_conn());
        assert_eq!(source, sub.get_source());
        let got_name = sub.get_dst();
        assert_eq!(dst, got_name);
    }

    fn test_publish_template(
        source: Name,
        dst: Name,
        identity: Option<&str>,
        flags: Option<SlimHeaderFlags>,
    ) {
        let mut builder = ProtoMessage::builder()
            .source(source.clone())
            .destination(dst.clone())
            .application_payload("str", "this is the content of the message".into());

        if let Some(id) = identity {
            builder = builder.identity(id);
        }

        if let Some(f) = flags.clone() {
            builder = builder.flags(f);
        }

        let pub_msg = builder.build_publish().unwrap();

        let flags = if flags.is_none() {
            Some(SlimHeaderFlags::default())
        } else {
            flags
        };

        assert!(pub_msg.is_publish());
        assert!(!pub_msg.is_subscribe());
        assert!(!pub_msg.is_unsubscribe());
        assert_eq!(flags.as_ref().unwrap().recv_from, pub_msg.get_recv_from());
        assert_eq!(flags.as_ref().unwrap().forward_to, pub_msg.get_forward_to());
        assert_eq!(None, pub_msg.try_get_incoming_conn());
        assert_eq!(source, pub_msg.get_source());
        let got_name = pub_msg.get_dst();
        assert_eq!(dst, got_name);
        assert_eq!(flags.as_ref().unwrap().fanout, pub_msg.get_fanout());
    }

    #[test]
    fn test_subscription() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = Name::from_strings(["org", "ns", "type"]).with_id(2);

        // simple
        test_subscription_template(true, source.clone(), dst.clone(), None, None);

        // with name id
        test_subscription_template(true, source.clone(), dst.clone(), None, None);

        // with recv from
        test_subscription_template(
            true,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );

        // with forward to
        test_subscription_template(
            true,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );
    }

    #[test]
    fn test_unsubscription() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = Name::from_strings(["org", "ns", "type"]).with_id(2);

        // simple
        test_subscription_template(false, source.clone(), dst.clone(), None, None);

        // with name id
        test_subscription_template(false, source.clone(), dst.clone(), None, None);

        // with recv from
        test_subscription_template(
            false,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );

        // with forward to
        test_subscription_template(
            false,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );
    }

    #[test]
    fn test_publish() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let mut dst = Name::from_strings(["org", "ns", "type"]);

        // simple
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default()),
        );

        // with name id
        dst.set_id(2);
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default()),
        );
        dst.reset_id();

        // with recv from
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );

        // with forward to
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );

        // with fanout
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_fanout(2)),
        );
    }

    #[test]
    fn test_conversions() {
        // Name to ProtoName
        let name = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let proto_name = ProtoName::from(&name);

        assert_eq!(
            proto_name.name.as_ref().unwrap().component_0,
            name.components()[0]
        );
        assert_eq!(
            proto_name.name.as_ref().unwrap().component_1,
            name.components()[1]
        );
        assert_eq!(
            proto_name.name.as_ref().unwrap().component_2,
            name.components()[2]
        );
        assert_eq!(
            proto_name.name.as_ref().unwrap().component_3,
            name.components()[3]
        );

        // ProtoName to Name
        let name_from_proto = Name::from(&proto_name);
        assert_eq!(
            name_from_proto.components()[0],
            proto_name.name.as_ref().unwrap().component_0
        );
        assert_eq!(
            name_from_proto.components()[1],
            proto_name.name.as_ref().unwrap().component_1
        );
        assert_eq!(
            name_from_proto.components()[2],
            proto_name.name.as_ref().unwrap().component_2
        );
        assert_eq!(
            name_from_proto.components()[3],
            proto_name.name.as_ref().unwrap().component_3
        );

        // ProtoMessage to ProtoSubscribe
        let dst = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let proto_subscribe = ProtoMessage::builder()
            .source(name.clone())
            .destination(dst.clone())
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .build_subscribe()
            .unwrap();
        let proto_subscribe = ProtoSubscribe::from(proto_subscribe);
        assert_eq!(proto_subscribe.header.as_ref().unwrap().get_source(), name);
        assert_eq!(proto_subscribe.header.as_ref().unwrap().get_dst(), dst,);

        // ProtoMessage to ProtoUnsubscribe
        let proto_unsubscribe = ProtoMessage::builder()
            .source(name.clone())
            .destination(dst.clone())
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .build_unsubscribe()
            .unwrap();
        let proto_unsubscribe = ProtoUnsubscribe::from(proto_unsubscribe);
        assert_eq!(
            proto_unsubscribe.header.as_ref().unwrap().get_source(),
            name
        );
        assert_eq!(proto_unsubscribe.header.as_ref().unwrap().get_dst(), dst);

        // ProtoMessage to ProtoPublish
        let proto_publish = ProtoMessage::builder()
            .source(name.clone())
            .destination(dst.clone())
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .application_payload("str", "this is the content of the message".into())
            .build_publish()
            .unwrap();
        let proto_publish = ProtoPublish::from(proto_publish);
        assert_eq!(proto_publish.header.as_ref().unwrap().get_source(), name);
        assert_eq!(proto_publish.header.as_ref().unwrap().get_dst(), dst);
    }

    #[test]
    fn test_panic() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = Name::from_strings(["org", "ns", "type"]).with_id(2);

        // panic if SLIM header is not found
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dst.clone())
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .build_subscribe()
            .unwrap();

        // let's try to convert it to a unsubscribe
        // this should panic because the message type is not unsubscribe
        let result = std::panic::catch_unwind(|| ProtoUnsubscribe::from(msg.clone()));
        assert!(result.is_err());

        // try to convert to publish
        // this should panic because the message type is not publish
        let result = std::panic::catch_unwind(|| ProtoPublish::from(msg.clone()));
        assert!(result.is_err());

        // finally make sure the conversion to subscribe works
        let result = std::panic::catch_unwind(|| ProtoSubscribe::from(msg));
        assert!(result.is_ok());
    }

    #[test]
    fn test_panic_header() {
        // create a unusual SLIM header
        let header = SlimHeader {
            source: None,
            destination: None,
            identity: String::new(),
            fanout: 0,
            version: version().to_string(),
            recv_from: None,
            forward_to: None,
            incoming_conn: None,
            error: None,
        };

        // the operations to retrieve source and destination should fail with panic
        let result = std::panic::catch_unwind(|| header.get_source());
        assert!(result.is_err());

        let result = std::panic::catch_unwind(|| header.get_dst());
        assert!(result.is_err());

        // The operations to retrieve recv_from and forward_to should not fail with panic
        let result = std::panic::catch_unwind(|| header.get_recv_from());
        assert!(result.is_ok());

        let result = std::panic::catch_unwind(|| header.get_forward_to());
        assert!(result.is_ok());

        // The operations to retrieve incoming_conn should not fail with panic
        let result = std::panic::catch_unwind(|| header.get_incoming_conn());
        assert!(result.is_ok());

        // The operations to retrieve error should not fail with panic
        let result = std::panic::catch_unwind(|| header.get_error());
        assert!(result.is_ok());
    }

    #[test]
    fn test_panic_session_header() {
        // create a unusual session header
        let header = SessionHeader::new(0, 0, 0, 0);

        // the operations to retrieve session_id and message_id should not fail with panic
        let result = std::panic::catch_unwind(|| header.get_session_id());
        assert!(result.is_ok());

        let result = std::panic::catch_unwind(|| header.get_message_id());
        assert!(result.is_ok());
    }

    #[test]
    fn test_panic_proto_message() {
        // create a unusual proto message
        let message = ProtoMessage {
            metadata: HashMap::new(),
            message_type: None,
        };

        // the operation to retrieve the header should fail with panic
        let result = std::panic::catch_unwind(|| message.get_slim_header());
        assert!(result.is_err());

        // the operation to retrieve the message type should fail with panic
        let result = std::panic::catch_unwind(|| message.get_type());
        assert!(result.is_err());

        // all the other ops should fail with panic as well as the header is not set
        let result = std::panic::catch_unwind(|| message.get_source());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| message.get_dst());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| message.get_recv_from());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| message.get_forward_to());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| message.get_incoming_conn());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| message.get_fanout());
        assert!(result.is_err());
    }

    #[test]
    fn test_service_type_to_int() {
        // Get total number of service types
        let total_service_types = SessionMessageType::Ping as i32;

        for i in 0..total_service_types {
            // int -> ServiceType
            let service_type =
                SessionMessageType::try_from(i).expect("failed to convert int to service type");
            let service_type_int = i32::from(service_type);
            assert_eq!(service_type_int, i32::from(service_type),);
        }

        // Test invalid conversion
        let invalid_service_type = SessionMessageType::try_from(total_service_types + 1);
        assert!(invalid_service_type.is_err());
    }

    #[test]
    fn test_proto_message_builder() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let dest = Name::from_strings(["org", "ns", "app"]).with_id(2);

        // Test basic publish message
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .application_payload("test", b"hello world".to_vec())
            .build_publish()
            .unwrap();

        assert!(msg.is_publish());
        assert_eq!(msg.get_source(), source);
        assert_eq!(msg.get_dst(), dest);

        // Test with session headers
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(SessionMessageType::Msg)
            .session_id(42)
            .message_id(100)
            .fanout(256)
            .application_payload("test", b"broadcast".to_vec())
            .build_publish()
            .unwrap();

        assert_eq!(msg.get_session_type(), ProtoSessionType::Multicast);
        assert_eq!(msg.get_id(), 100);
        assert_eq!(msg.get_fanout(), 256);

        // Test with metadata
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .metadata("key1", "value1")
            .metadata("key2", "value2")
            .application_payload("test", vec![1, 2, 3])
            .build_publish()
            .unwrap();

        assert_eq!(msg.get_metadata("key1"), Some(&"value1".to_string()));
        assert_eq!(msg.get_metadata("key2"), Some(&"value2".to_string()));

        // Test subscribe message
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .recv_from(10)
            .build_subscribe()
            .unwrap();

        assert!(msg.is_subscribe());
        assert_eq!(msg.get_recv_from(), Some(10));

        // Test unsubscribe message
        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .forward_to(20)
            .build_unsubscribe()
            .unwrap();

        assert!(msg.is_unsubscribe());
        assert_eq!(msg.get_forward_to(), Some(20));
    }

    #[test]
    fn test_command_payload_builder() {
        let dest = Name::from_strings(["org", "ns", "app"]);

        // Test discovery request
        let payload = CommandPayload::builder().discovery_request(Some(dest.clone()));
        let extracted = payload.as_discovery_request_payload().unwrap();
        assert!(extracted.destination.is_some());

        // Test discovery reply
        let payload = CommandPayload::builder().discovery_reply();
        assert!(payload.as_discovery_reply_payload().is_ok());

        // Test join request
        let payload = CommandPayload::builder().join_request(
            true,
            Some(5),
            Some(Duration::from_secs(10)),
            Some(dest.clone()),
        );
        let extracted = payload.as_join_request_payload().unwrap();
        assert!(extracted.enable_mls);
        assert!(extracted.timer_settings.is_some());

        // Test join reply
        let participant = Participant::new(dest.clone(), ParticipantSettings::bidirectional());
        let payload =
            CommandPayload::builder().join_reply(Some(vec![1, 2, 3]), participant.clone());
        let extracted = payload.as_join_reply_payload().unwrap();
        assert_eq!(extracted.key_package, Some(vec![1, 2, 3]));
        assert_eq!(extracted.participant, Some(participant));

        // Test leave request
        let payload = CommandPayload::builder().leave_request(Some(dest.clone()));
        assert!(payload.as_leave_request_payload().is_ok());

        // Test leave reply
        let payload = CommandPayload::builder().leave_reply();
        assert!(payload.as_leave_reply_payload().is_ok());

        // Test group add
        let participant = Participant::new(dest.clone(), ParticipantSettings::bidirectional());
        let participants = vec![participant.clone()];
        let payload =
            CommandPayload::builder().group_add(participant.clone(), participants.clone(), None);
        let extracted = payload.as_group_add_payload().unwrap();
        assert_eq!(extracted.new_participant, Some(participant));
        assert_eq!(extracted.participants, participants);

        // Test group remove
        let payload =
            CommandPayload::builder().group_remove(dest.clone(), vec![dest.clone()], None);
        let extracted = payload.as_group_remove_payload().unwrap();
        assert!(extracted.removed_participant.is_some());

        // Test group welcome
        let payload = CommandPayload::builder().group_welcome(participants.clone(), None);
        let extracted = payload.as_welcome_payload().unwrap();
        assert!(!extracted.participants.is_empty());

        // Test group proposal
        let payload = CommandPayload::builder().group_proposal(Some(dest.clone()), vec![4, 5, 6]);
        let extracted = payload.as_group_proposal_payload().unwrap();
        assert_eq!(extracted.mls_proposal, vec![4, 5, 6]);

        // Test group ack
        let payload = CommandPayload::builder().group_ack();
        assert!(payload.as_group_ack_payload().is_ok());

        // Test group nack
        let payload = CommandPayload::builder().group_nack();
        assert!(payload.as_group_nack_payload().is_ok());

        // Test ping
        let payload = CommandPayload::builder().ping();
        assert!(payload.as_ping_payload().is_ok());
    }

    #[test]
    fn test_builder_with_command_payload() {
        let source = Name::from_strings(["org", "ns", "type"]).with_id(1);
        let dest = Name::from_strings(["org", "ns", "app"]).with_id(2);

        let cmd_payload = CommandPayload::builder().discovery_request(Some(dest.clone()));

        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(SessionMessageType::DiscoveryRequest)
            .session_id(1)
            .command_payload(cmd_payload)
            .build_publish()
            .unwrap();

        assert!(msg.is_publish());
        assert_eq!(
            msg.get_session_message_type(),
            SessionMessageType::DiscoveryRequest
        );

        // Verify we can extract the payload
        let extracted = msg.extract_discovery_request().unwrap();
        assert!(extracted.destination.is_some());
    }

    #[test]
    fn test_validate_link_without_link_type() {
        let link = ProtoLink { link_type: None };
        let msg = ProtoMessage::new(HashMap::new(), ProtoLinkMessageType(link));
        assert!(matches!(msg.validate(), Err(MessageError::LinkTypeNotSet)));
    }

    #[test]
    fn test_validate_link_with_link_type() {
        let link = ProtoLink {
            link_type: Some(ProtoLinkType::LinkNegotiation(LinkNegotiationPayload {
                link_id: "abc".into(),
                slim_version: "1.0.0".into(),
                is_reply: false,
            })),
        };
        let msg = ProtoMessage::new(HashMap::new(), ProtoLinkMessageType(link));
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_build_link_negotiation_request() {
        let msg = ProtoMessage::builder().build_link_negotiation("my-id", "1.2.3", false);
        assert!(msg.is_link());
        assert!(!msg.is_publish());
        assert!(!msg.is_subscribe());
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_build_link_negotiation_reply() {
        let msg = ProtoMessage::builder().build_link_negotiation("my-id", "1.2.3", true);
        assert!(msg.is_link());
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_participant_settings_convenience_methods() {
        let bidirectional = ParticipantSettings::bidirectional();
        assert!(bidirectional.sends_data);
        assert!(bidirectional.receives_data);
        assert!(bidirectional.is_sender());
        assert!(bidirectional.is_receiver());

        let send_only = ParticipantSettings::send_only();
        assert!(send_only.sends_data);
        assert!(!send_only.receives_data);
        assert!(send_only.is_sender());
        assert!(!send_only.is_receiver());

        let receive_only = ParticipantSettings::receive_only();
        assert!(!receive_only.sends_data);
        assert!(receive_only.receives_data);
        assert!(!receive_only.is_sender());
        assert!(receive_only.is_receiver());
    }
}
