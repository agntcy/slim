// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use slim_version::version;
use thiserror::Error;
use twox_hash::XxHash64;

use crate::dataplane::proto::v1::command_payload::CommandPayloadType;
use crate::dataplane::proto::v1::content::ContentType;
use crate::dataplane::proto::v1::link::LinkType as ProtoLinkType;
use crate::dataplane::proto::v1::message::MessageType;
use crate::dataplane::proto::v1::message::MessageType::Link as ProtoLinkMessageType;
use crate::dataplane::proto::v1::message::MessageType::Publish as ProtoPublishType;
use crate::dataplane::proto::v1::message::MessageType::Subscribe as ProtoSubscribeType;
use crate::dataplane::proto::v1::message::MessageType::SubscriptionAck as ProtoSubscriptionAckType;
use crate::dataplane::proto::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
use crate::dataplane::proto::v1::{
    ApplicationPayload, CommandPayload, Content, DiscoveryReplyPayload, DiscoveryRequestPayload,
    EncodedName, GroupAckPayload, GroupAddPayload, GroupClosePayload, GroupNackPayload,
    GroupProposalPayload, GroupRemovePayload, GroupWelcomePayload, JoinReplyPayload,
    JoinRequestPayload, LeaveReplyPayload, LeaveRequestPayload, Link as ProtoLink,
    LinkConnectionType, LinkNegotiationPayload, Message as ProtoMessage, MlsPayload,
    MlsSettings as ProtoMlsSettings, Name as ProtoName, NameId, Participant, ParticipantSettings,
    PingPayload, Publish as ProtoPublish, SessionHeader, SessionMessageType,
    SessionType as ProtoSessionType, SlimHeader, StringName, Subscribe as ProtoSubscribe,
    SubscriptionAck as ProtoSubscriptionAck, TimerSettings, Unsubscribe as ProtoUnsubscribe,
};

fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut hasher = XxHash64::default();
    t.hash(&mut hasher);
    hasher.finish()
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    #[error("invalid name id format: {0}")]
    InvalidNameIdFormat(String),
    #[error("invalid name format: {0}")]
    InvalidNameFormat(String),
}

/// DELETE_GROUP indicates that the entire group is being closed.
/// The moderator sets this metadata on the leave message sent to all participants
/// when a channel deletion is requested.
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

/// Default TTL value for messages that do not have an explicit TTL set.
pub const DEFAULT_TTL: u32 = 8;

#[derive(Error, Debug, PartialEq)]
pub enum MessageError {
    #[error("SLIM header not found")]
    SlimHeaderNotFound,
    #[error("source not found")]
    SourceNotFound,
    #[error("source encoded name not found")]
    SourceEncodedNameNotFound,
    #[error("destination not found")]
    DestinationNotFound,
    #[error("destination encoded name not found")]
    DestinationEncodedNameNotFound,
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
    #[error("builder error: source is required")]
    BuilderErrorSourceRequired,
    #[error("builder error: destination is required")]
    BuilderErrorDestinationRequired,
    #[error("participant name not found")]
    ParticipantNameNotFound,
    #[error("participant settings not found")]
    ParticipantSettingsNotFound,
}

/// Struct grouping the SLIM header flags for convenience.
#[derive(Debug, Clone)]
pub struct SlimHeaderFlags {
    pub fanout: u32,
    pub recv_from: Option<u64>,
    pub forward_to: Option<u64>,
    pub incoming_conn: Option<u64>,
    pub error: Option<bool>,
    pub ttl: u32,
}

impl Default for SlimHeaderFlags {
    fn default() -> Self {
        Self {
            fanout: 1,
            recv_from: None,
            forward_to: None,
            incoming_conn: None,
            error: None,
            ttl: DEFAULT_TTL,
        }
    }
}

impl Display for SlimHeaderFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fanout: {}, recv_from: {:?}, forward_to: {:?}, incoming_conn: {:?}, error: {:?}, ttl: {:?}",
            self.fanout, self.recv_from, self.forward_to, self.incoming_conn, self.error, self.ttl
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
            ttl: DEFAULT_TTL,
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

    pub fn with_ttl(self, ttl: u32) -> Self {
        Self { ttl, ..self }
    }
}

impl NameId {
    pub const NULL_COMPONENT: u128 = u128::MAX;
    pub const RESERVED_IDS: u128 = 50;

    pub const fn is_reserved_id(id: u128) -> bool {
        id >= (u128::MAX - Self::RESERVED_IDS)
    }
}

impl std::fmt::Display for NameId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: String = (*self).into();
        write!(f, "{}", s)
    }
}

impl From<u128> for NameId {
    fn from(id: u128) -> Self {
        NameId {
            id_0: (id >> 64) as u64,
            id_1: (id & 0xFFFFFFFFFFFFFFFF) as u64,
        }
    }
}

impl From<NameId> for u128 {
    fn from(name_id: NameId) -> Self {
        (name_id.id_0 as u128) << 64 | (name_id.id_1 as u128)
    }
}

impl TryFrom<String> for NameId {
    type Error = NameError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "NULL_COMPONENT" => Ok(Self::from(Self::NULL_COMPONENT)),
            _ => {
                if let Ok(uuid) = uuid::Uuid::parse_str(&s) {
                    return Ok(Self::from(uuid.as_u128()));
                }
                Err(NameError::InvalidNameIdFormat(s))
            }
        }
    }
}

impl From<NameId> for String {
    fn from(name_id: NameId) -> String {
        let val: u128 = name_id.into();
        match val {
            NameId::NULL_COMPONENT => "NULL_COMPONENT".to_string(),
            id => uuid::Uuid::from_u128(id).to_string(),
        }
    }
}

impl EncodedName {
    pub fn id(&self) -> u128 {
        self.name_id
            .as_ref()
            .map_or(NameId::NULL_COMPONENT, |nid| (*nid).into())
    }

    pub fn string_id(&self) -> String {
        self.name_id
            .as_ref()
            .map_or("NULL_COMPONENT".to_string(), |nid| (*nid).into())
    }
}

impl ProtoName {
    pub fn from_strings(components: [impl Into<String>; 3]) -> Self {
        let [s0, s1, s2] = components.map(Into::into);
        Self {
            name: Some(EncodedName {
                component_0: calculate_hash(&s0),
                component_1: calculate_hash(&s1),
                component_2: calculate_hash(&s2),
                name_id: Some(NameId::from(NameId::NULL_COMPONENT)),
            }),
            str_name: Some(StringName {
                str_component_0: s0,
                str_component_1: s1,
                str_component_2: s2,
            }),
        }
    }

    pub fn with_id(mut self, id: u128) -> Self {
        self.name.as_mut().expect("encoded name missing").name_id = Some(NameId::from(id));
        self
    }

    pub fn id(&self) -> u128 {
        self.name.as_ref().expect("encoded name missing").id()
    }

    pub fn string_id(&self) -> String {
        self.name
            .as_ref()
            .expect("encoded name missing")
            .string_id()
    }

    pub fn name_id(&self) -> Option<NameId> {
        self.name.as_ref().expect("encoded name missing").name_id
    }

    pub fn has_id(&self) -> bool {
        self.id() != NameId::NULL_COMPONENT
    }

    pub fn set_id(&mut self, id: u128) {
        self.name.as_mut().expect("encoded name missing").name_id = Some(NameId::from(id));
    }

    pub fn reset_id(&mut self) {
        self.name.as_mut().expect("encoded name missing").name_id =
            Some(NameId::from(NameId::NULL_COMPONENT));
    }

    pub fn match_prefix(&self, other: &ProtoName) -> bool {
        let a = self.name.as_ref().expect("encoded name missing");
        let b = other.name.as_ref().expect("encoded name missing");
        a.component_0 == b.component_0
            && a.component_1 == b.component_1
            && a.component_2 == b.component_2
    }

    pub fn str_components(&self) -> (&str, &str, &str) {
        let s = self.str_name.as_ref().expect("string name missing");
        (&s.str_component_0, &s.str_component_1, &s.str_component_2)
    }

    pub fn parse_name(s: &str) -> Result<ProtoName, NameError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(NameError::InvalidNameFormat(s.to_string()));
        }
        let parts: Vec<&str> = trimmed.split('/').map(str::trim).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            return Err(NameError::InvalidNameFormat(s.to_string()));
        }
        Ok(ProtoName::from_strings([parts[0], parts[1], parts[2]]))
    }
}

impl std::fmt::Display for ProtoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) = &self.str_name {
            write!(
                f,
                "{}/{}/{}/{}",
                s.str_component_0,
                s.str_component_1,
                s.str_component_2,
                self.name
                    .as_ref()
                    .and_then(|n| n.name_id)
                    .map_or("NULL_COMPONENT".to_string(), |id| id.to_string())
            )
        } else if let Some(enc) = &self.name {
            write!(
                f,
                "{}/{}/{}/{}",
                enc.component_0,
                enc.component_1,
                enc.component_2,
                enc.name_id
                    .as_ref()
                    .map_or("NULL_COMPONENT".to_string(), |id| id.to_string())
            )
        } else {
            write!(f, "<empty>")
        }
    }
}

impl ParticipantSettings {
    pub fn bidirectional() -> Self {
        Self {
            sends_data: true,
            receives_data: true,
        }
    }

    pub fn send_only() -> Self {
        Self {
            sends_data: true,
            receives_data: false,
        }
    }

    pub fn receive_only() -> Self {
        Self {
            sends_data: false,
            receives_data: true,
        }
    }

    pub fn is_sender(&self) -> bool {
        self.sends_data
    }

    pub fn is_receiver(&self) -> bool {
        self.receives_data
    }
}

impl Participant {
    pub fn new(name: ProtoName, settings: ParticipantSettings) -> Self {
        Self {
            name: Some(name),
            settings: Some(settings),
        }
    }

    pub fn get_name(&self) -> Result<ProtoName, MessageError> {
        match &self.name {
            Some(name) => Ok(name.clone()),
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

impl SlimHeader {
    pub fn new(
        source: ProtoName,
        destination: ProtoName,
        identity: &str,
        flags: Option<SlimHeaderFlags>,
    ) -> Self {
        let flags = flags.unwrap_or_default();
        Self {
            source: Some(source),
            destination: Some(destination),
            identity: identity.to_string(),
            fanout: flags.fanout,
            version: version().to_string(),
            recv_from: flags.recv_from,
            forward_to: flags.forward_to,
            incoming_conn: flags.incoming_conn,
            error: flags.error,
            header_mac: None,
            ttl: flags.ttl,
            e2e_header_sig: None,
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

    pub fn get_source(&self) -> ProtoName {
        self.source.clone().expect("source not found")
    }

    pub fn get_encoded_source(&self) -> EncodedName {
        self.source
            .as_ref()
            .expect("source not found")
            .name
            .expect("source encoded name not found")
    }

    pub fn get_dst(&self) -> ProtoName {
        self.destination.clone().expect("destination not found")
    }

    pub fn get_encoded_dst(&self) -> EncodedName {
        self.destination
            .as_ref()
            .expect("destination not found")
            .name
            .expect("destination encoded name not found")
    }

    pub fn get_identity(&self) -> String {
        self.identity.clone()
    }

    pub fn get_version(&self) -> String {
        self.version.clone()
    }

    pub fn set_source(&mut self, source: ProtoName) {
        self.source = Some(source);
    }

    pub fn set_destination(&mut self, dst: ProtoName) {
        self.destination = Some(dst);
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

    pub fn get_ttl(&self) -> u32 {
        self.ttl
    }

    pub fn set_ttl(&mut self, ttl: u32) {
        self.ttl = ttl;
    }

    pub fn decrement_ttl(&mut self) -> u32 {
        self.ttl = self.ttl.saturating_sub(1);
        self.ttl
    }

    pub fn get_connections(&self) -> (u64, Option<u64>, Option<u64>) {
        let incoming = self
            .get_incoming_conn()
            .expect("incoming connection not found");
        (incoming, self.get_recv_from(), self.get_forward_to())
    }
}

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

impl SessionMessageType {
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

    pub fn is_post_session_control(&self) -> bool {
        matches!(
            self,
            SessionMessageType::LeaveRequest
                | SessionMessageType::LeaveReply
                | SessionMessageType::GroupAdd
                | SessionMessageType::GroupRemove
                | SessionMessageType::GroupClose
                | SessionMessageType::GroupProposal
                | SessionMessageType::GroupAck
                | SessionMessageType::GroupNack
                | SessionMessageType::Ping
        )
    }
}

impl ProtoSubscribe {
    fn new(
        source: ProtoName,
        dst: ProtoName,
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

impl From<ProtoMessage> for ProtoSubscribe {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoSubscribeType(s)) => s,
            _ => panic!("message type is not subscribe"),
        }
    }
}

impl ProtoUnsubscribe {
    fn new(
        source: ProtoName,
        dst: ProtoName,
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

impl From<ProtoMessage> for ProtoUnsubscribe {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoUnsubscribeType(u)) => u,
            _ => panic!("message type is not unsubscribe"),
        }
    }
}

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
        self.header.as_ref().expect("SLIM header missing")
    }

    pub fn get_session_header(&self) -> &SessionHeader {
        self.session.as_ref().expect("session header missing")
    }

    pub fn get_slim_header_as_mut(&mut self) -> &mut SlimHeader {
        self.header.as_mut().expect("SLIM header missing")
    }

    pub fn get_session_header_as_mut(&mut self) -> &mut SessionHeader {
        self.session.as_mut().expect("session header missing")
    }

    pub fn get_payload(&self) -> &Content {
        self.msg.as_ref().expect("payload missing")
    }

    pub fn set_payload(&mut self, payload: Content) {
        self.msg = Some(payload);
    }

    pub fn is_command(&self) -> bool {
        match &self
            .get_payload()
            .content_type
            .as_ref()
            .expect("content missing")
        {
            ContentType::AppPayload(_) => false,
            ContentType::CommandPayload(_) => true,
        }
    }

    pub fn get_application_payload(&self) -> &ApplicationPayload {
        match self
            .get_payload()
            .content_type
            .as_ref()
            .expect("content missing")
        {
            ContentType::AppPayload(application_payload) => application_payload,
            ContentType::CommandPayload(_) => panic!("the payload is not an application payload"),
        }
    }

    pub fn get_command_payload(&self) -> &CommandPayload {
        match self
            .get_payload()
            .content_type
            .as_ref()
            .expect("content missing")
        {
            ContentType::AppPayload(_) => panic!("the payload is not a command payload"),
            ContentType::CommandPayload(command_payload) => command_payload,
        }
    }
}

impl From<ProtoMessage> for ProtoPublish {
    fn from(message: ProtoMessage) -> Self {
        match message.message_type {
            Some(ProtoPublishType(p)) => p,
            _ => panic!("message type is not publish"),
        }
    }
}

macro_rules! impl_payload_extractors {
    ($($method_name:ident => $getter_method:ident($payload_type:ty)),* $(,)?) => {
        $(
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
        match &slim_header.source {
            None => return Err(MessageError::SourceNotFound),
            Some(src) if src.name.is_none() => return Err(MessageError::SourceEncodedNameNotFound),
            _ => {}
        }
        match &slim_header.destination {
            None => return Err(MessageError::DestinationNotFound),
            Some(dst) if dst.name.is_none() => {
                return Err(MessageError::DestinationEncodedNameNotFound);
            }
            _ => {}
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

    pub fn insert_metadata(&mut self, key: String, val: String) {
        self.metadata.insert(key, val);
    }

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
        for (k, v) in &map {
            self.insert_metadata(k.to_string(), v.to_string());
        }
    }

    pub fn get_slim_header(&self) -> &SlimHeader {
        match &self.message_type {
            Some(ProtoPublishType(publish)) => {
                publish.header.as_ref().expect("SLIM header missing")
            }
            Some(ProtoSubscribeType(sub)) => sub.header.as_ref().expect("SLIM header missing"),
            Some(ProtoUnsubscribeType(unsub)) => {
                unsub.header.as_ref().expect("SLIM header missing")
            }
            Some(ProtoLinkMessageType(_)) | Some(ProtoSubscriptionAckType(_)) | None => {
                panic!("SLIM header not found")
            }
        }
    }

    pub fn get_slim_header_mut(&mut self) -> &mut SlimHeader {
        match &mut self.message_type {
            Some(ProtoPublishType(publish)) => {
                publish.header.as_mut().expect("SLIM header missing")
            }
            Some(ProtoSubscribeType(sub)) => sub.header.as_mut().expect("SLIM header missing"),
            Some(ProtoUnsubscribeType(unsub)) => {
                unsub.header.as_mut().expect("SLIM header missing")
            }
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
            Some(ProtoPublishType(publish)) => {
                publish.session.as_ref().expect("session header missing")
            }
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => panic!("session header not found"),
        }
    }

    pub fn get_session_header_mut(&mut self) -> &mut SessionHeader {
        match &mut self.message_type {
            Some(ProtoPublishType(publish)) => {
                publish.session.as_mut().expect("session header missing")
            }
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

    pub fn get_source(&self) -> ProtoName {
        self.get_slim_header().get_source()
    }

    pub fn get_encoded_source(&self) -> EncodedName {
        self.get_slim_header().get_encoded_source()
    }

    pub fn get_dst(&self) -> ProtoName {
        self.get_slim_header().get_dst()
    }

    pub fn get_encoded_dst(&self) -> EncodedName {
        self.get_slim_header().get_encoded_dst()
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
        self.get_slim_header()
            .get_incoming_conn()
            .expect("incoming connection not found")
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
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => panic!("payload not found"),
        }
    }

    pub fn set_payload(&mut self, payload: Content) {
        match &mut self.message_type {
            Some(ProtoPublishType(p)) => p.set_payload(payload),
            Some(ProtoSubscribeType(_))
            | Some(ProtoUnsubscribeType(_))
            | Some(ProtoLinkMessageType(_))
            | Some(ProtoSubscriptionAckType(_))
            | None => panic!("no payload allowed"),
        }
    }

    pub fn get_session_message_type(&self) -> SessionMessageType {
        self.get_session_header().session_message_type()
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

    pub fn get_ttl(&self) -> u32 {
        self.get_slim_header().get_ttl()
    }

    pub fn set_ttl(&mut self, ttl: u32) {
        self.get_slim_header_mut().set_ttl(ttl);
    }

    pub fn decrement_ttl(&mut self) -> u32 {
        self.get_slim_header_mut().decrement_ttl()
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

    pub fn get_link_negotiation_payload(&self) -> Option<LinkNegotiationPayload> {
        match &self.message_type {
            Some(ProtoLinkMessageType(link)) => match &link.link_type {
                Some(ProtoLinkType::LinkNegotiation(payload)) => Some(payload.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn is_subscription_ack(&self) -> bool {
        matches!(self.get_type(), MessageType::SubscriptionAck(_))
    }

    pub fn is_traceable(&self) -> bool {
        !self.is_link() && !self.is_subscription_ack()
    }

    pub fn get_subscription_ack(&self) -> &ProtoSubscriptionAck {
        match &self.message_type {
            Some(ProtoSubscriptionAckType(ack)) => ack,
            _ => panic!("message type is not subscription_ack"),
        }
    }

    pub fn get_subscription_id(&self) -> Option<u64> {
        match &self.message_type {
            Some(ProtoSubscribeType(s)) if s.subscription_id != 0 => Some(s.subscription_id),
            Some(ProtoUnsubscribeType(u)) if u.subscription_id != 0 => Some(u.subscription_id),
            _ => None,
        }
    }

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

    pub fn set_subscription_id(&mut self, subscription_id: u64) {
        match &mut self.message_type {
            Some(ProtoSubscribeType(s)) => s.subscription_id = subscription_id,
            Some(ProtoUnsubscribeType(u)) => u.subscription_id = subscription_id,
            _ => {}
        }
    }

    pub fn extract_command_payload(&self) -> Result<&CommandPayload, MessageError> {
        self.get_payload()
            .ok_or(MessageError::ContentTypeNotSet)?
            .as_command_payload()
    }

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

    pub fn builder() -> ProtoMessageBuilder {
        ProtoMessageBuilder::new()
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

macro_rules! impl_command_payload_getters {
    ($($method_name:ident => $variant:ident($payload_type:ty)),* $(,)?) => {
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

    pub fn builder() -> CommandPayloadBuilder {
        CommandPayloadBuilder::new()
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

pub struct CommandPayloadBuilder;

impl CommandPayloadBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn discovery_request(self) -> CommandPayload {
        let payload = DiscoveryRequestPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::DiscoveryRequest(payload)),
        }
    }

    pub fn discovery_reply(self) -> CommandPayload {
        let payload = DiscoveryReplyPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::DiscoveryReply(payload)),
        }
    }

    #[allow(deprecated)]
    pub fn join_request(
        self,
        max_retries: Option<u32>,
        timer_duration: Option<Duration>,
        data_channel: Option<ProtoName>,
        control_channel: Option<ProtoName>,
        mls_settings: Option<ProtoMlsSettings>,
    ) -> CommandPayload {
        let timer_settings = match (timer_duration, max_retries) {
            (Some(t), Some(m)) => Some(TimerSettings {
                timeout: t.as_millis() as u32,
                max_retries: m,
            }),
            _ => None,
        };

        let payload = JoinRequestPayload {
            timer_settings,
            channel: data_channel,
            control: control_channel,
            mls_settings,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::JoinRequest(payload)),
        }
    }

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

    pub fn leave_request(self) -> CommandPayload {
        let payload = LeaveRequestPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::LeaveRequest(payload)),
        }
    }

    pub fn leave_reply(self) -> CommandPayload {
        let payload = LeaveReplyPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::LeaveReply(payload)),
        }
    }

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

    pub fn group_remove(
        self,
        removed_participant: ProtoName,
        participants: Vec<ProtoName>,
        mls: Option<MlsPayload>,
    ) -> CommandPayload {
        let payload = GroupRemovePayload {
            removed_participant: Some(removed_participant),
            participants,
            mls,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupRemove(payload)),
        }
    }

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

    pub fn group_close(self, participants: Vec<ProtoName>) -> CommandPayload {
        let payload = GroupClosePayload { participants };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupClose(payload)),
        }
    }

    pub fn group_proposal(
        self,
        source: Option<ProtoName>,
        mls_proposal: Vec<u8>,
    ) -> CommandPayload {
        let payload = GroupProposalPayload {
            source,
            mls_proposal,
        };
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupProposal(payload)),
        }
    }

    pub fn group_ack(self) -> CommandPayload {
        let payload = GroupAckPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupAck(payload)),
        }
    }

    pub fn group_nack(self) -> CommandPayload {
        let payload = GroupNackPayload {};
        CommandPayload {
            command_payload_type: Some(CommandPayloadType::GroupNack(payload)),
        }
    }

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

pub struct ProtoMessageBuilder {
    source: Option<ProtoName>,
    destination: Option<ProtoName>,
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

    pub fn source(mut self, source: ProtoName) -> Self {
        self.source = Some(source);
        self
    }

    pub fn destination(mut self, destination: ProtoName) -> Self {
        self.destination = Some(destination);
        self
    }

    pub fn identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    pub fn flags(mut self, flags: SlimHeaderFlags) -> Self {
        self.flags = Some(flags);
        self
    }

    pub fn fanout(mut self, fanout: u32) -> Self {
        self.flags.get_or_insert_default().fanout = fanout;
        self
    }

    pub fn recv_from(mut self, recv_from: u64) -> Self {
        self.flags.get_or_insert_default().recv_from = Some(recv_from);
        self
    }

    pub fn forward_to(mut self, forward_to: u64) -> Self {
        self.flags.get_or_insert_default().forward_to = Some(forward_to);
        self
    }

    pub fn incoming_conn(mut self, incoming_conn: u64) -> Self {
        self.flags.get_or_insert_default().incoming_conn = Some(incoming_conn);
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.flags.get_or_insert_default().error = Some(error);
        self
    }

    pub fn ttl(mut self, ttl: u32) -> Self {
        self.flags.get_or_insert_default().ttl = ttl;
        self
    }

    pub fn session_type(mut self, session_type: ProtoSessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    pub fn session_message_type(mut self, session_message_type: SessionMessageType) -> Self {
        self.session_message_type = Some(session_message_type);
        self
    }

    pub fn session_id(mut self, session_id: u32) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn message_id(mut self, message_id: u32) -> Self {
        self.message_id = Some(message_id);
        self
    }

    pub fn payload(mut self, payload: Content) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn application_payload(mut self, payload_type: &str, blob: Vec<u8>) -> Self {
        let app_payload = ApplicationPayload::new(payload_type, blob);
        self.payload = Some(app_payload.as_content());
        self
    }

    pub fn command_payload(mut self, payload: CommandPayload) -> Self {
        self.payload = Some(payload.as_content());
        self
    }

    pub fn with_slim_header(mut self, header: SlimHeader) -> Self {
        if let Some(src) = header.source.clone() {
            self.source = Some(src);
        }
        if let Some(dst) = header.destination.clone() {
            self.destination = Some(dst);
        }
        if !header.identity.is_empty() {
            self.identity = Some(header.identity.clone());
        }

        self.flags = Some(SlimHeaderFlags {
            fanout: header.fanout,
            recv_from: header.recv_from,
            forward_to: header.forward_to,
            incoming_conn: header.incoming_conn,
            error: header.error,
            ttl: header.ttl,
        });
        self
    }

    pub fn with_session_header(mut self, header: SessionHeader) -> Self {
        self.session_type = Some(header.session_type());
        self.session_message_type = Some(header.session_message_type());
        self.session_id = Some(header.session_id);
        self.message_id = Some(header.message_id);
        self
    }

    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn metadata_map(mut self, map: HashMap<String, String>) -> Self {
        self.metadata.extend(map);
        self
    }

    pub fn subscription_id(mut self, id: u64) -> Self {
        self.subscription_id = Some(id);
        self
    }

    pub fn build_publish(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let slim_header = Some(SlimHeader::new(
            source,
            destination,
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
        Ok(ProtoMessage::new(self.metadata, ProtoPublishType(publish)))
    }

    pub fn build_subscribe(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let mut subscribe =
            ProtoSubscribe::new(source, destination, self.identity.as_deref(), self.flags);
        subscribe.subscription_id = self.subscription_id.unwrap_or_default();

        Ok(ProtoMessage::new(
            self.metadata,
            ProtoSubscribeType(subscribe),
        ))
    }

    pub fn build_unsubscribe(self) -> Result<ProtoMessage, MessageError> {
        let source = self
            .source
            .ok_or(MessageError::BuilderErrorSourceRequired)?;
        let destination = self
            .destination
            .ok_or(MessageError::BuilderErrorDestinationRequired)?;

        let mut unsubscribe =
            ProtoUnsubscribe::new(source, destination, self.identity.as_deref(), self.flags);
        unsubscribe.subscription_id = self.subscription_id.unwrap_or_default();

        Ok(ProtoMessage::new(
            self.metadata,
            ProtoUnsubscribeType(unsubscribe),
        ))
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn build_link_negotiation(
        self,
        link_id: impl Into<String>,
        slim_version: impl Into<String>,
        is_reply: bool,
        link_ecdh_public_key: Option<Vec<u8>>,
        link_kem_payload: Option<Vec<u8>>,
        connection_type: LinkConnectionType,
        node_id: impl Into<String>,
        deployment_name: impl Into<String>,
    ) -> ProtoMessage {
        let link = ProtoLink {
            link_type: Some(ProtoLinkType::LinkNegotiation(LinkNegotiationPayload {
                link_id: link_id.into(),
                slim_version: slim_version.into(),
                is_reply,
                link_ecdh_public_key: link_ecdh_public_key.unwrap_or_default(),
                connection_type: connection_type.into(),
                node_id: node_id.into(),
                deployment_name: deployment_name.into(),
                link_kem_payload,
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

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn test_proto_name_from_strings() {
        let n1 = ProtoName::from_strings(["Org", "Default", "App"]);
        let n2 = ProtoName::from_strings(["Org", "Default", "App"]);
        assert_eq!(n1, n2);
        let n3 = ProtoName::from_strings(["Other", "Default", "App"]);
        assert_ne!(n1, n3);
    }

    #[test]
    fn test_proto_name_with_id() {
        let n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        assert_eq!(n.id(), 42);
        assert!(n.has_id());
    }

    #[test]
    fn test_proto_name_reset_id() {
        let mut n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        n.reset_id();
        assert_eq!(n.id(), NameId::NULL_COMPONENT);
        assert!(!n.has_id());
    }

    #[test]
    fn test_proto_name_match_prefix() {
        let n1 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(1);
        let n2 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(999);
        assert!(n1.match_prefix(&n2));

        let n3 = ProtoName::from_strings(["Other", "Default", "App"]).with_id(1);
        assert!(!n1.match_prefix(&n3));
    }

    #[test]
    fn test_proto_name_str_components() {
        let n = ProtoName::from_strings(["org", "ns", "app"]);
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_proto_name_hash_stability() {
        let proto = ProtoName::from_strings(["Org", "Default", "App"]).with_id(7);
        let enc = proto.name.unwrap();
        let proto2 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(7);
        let enc2 = proto2.name.unwrap();
        assert_eq!(enc, enc2);
        assert_eq!(enc.id(), 7);
    }

    #[test]
    fn test_name_id_roundtrip() {
        let values = [0u128, 1, 42, u128::MAX / 2, u128::MAX - 4];
        for v in values {
            let nid = NameId::from(v);
            let result: u128 = nid.into();
            assert_eq!(result, v, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_name_id_from_string_valid_uuid() {
        let nid = NameId::try_from("00000000-0000-0000-0000-00000000002a".to_string())
            .expect("valid UUID string should parse");
        let result: u128 = nid.into();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_name_id_from_string_invalid() {
        assert!(NameId::try_from("not-a-uuid".to_string()).is_err());
        assert!(NameId::try_from("".to_string()).is_err());
    }

    #[test]
    fn test_name_id_display_uuid() {
        let nid = NameId::from(42);
        assert_eq!(nid.to_string(), "00000000-0000-0000-0000-00000000002a");
    }

    #[test]
    fn test_name_id_display_reserved() {
        let str_nid: String = NameId::from(NameId::NULL_COMPONENT).into();
        assert_eq!(str_nid, "NULL_COMPONENT");
    }

    #[test]
    fn test_name_id_is_reserved() {
        assert!(NameId::is_reserved_id(NameId::NULL_COMPONENT));
        assert!(!NameId::is_reserved_id(0));
        assert!(!NameId::is_reserved_id(42));
    }

    #[test]
    fn test_proto_name_default_id_is_null_component() {
        let n = ProtoName::from_strings(["a", "b", "c"]);
        assert_eq!(n.id(), NameId::NULL_COMPONENT);
        assert!(!n.has_id());
    }

    #[test]
    fn test_proto_name_set_id() {
        let mut n = ProtoName::from_strings(["a", "b", "c"]);
        n.set_id(123);
        assert_eq!(n.id(), 123);
        assert!(n.has_id());
    }

    #[test]
    fn test_proto_name_string_id() {
        let n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        assert_eq!(n.string_id(), "00000000-0000-0000-0000-00000000002a");
    }

    #[test]
    fn test_proto_name_display_with_id() {
        let n = ProtoName::from_strings(["org", "ns", "app"]).with_id(1);
        let s = format!("{}", n);
        assert_eq!(s, "org/ns/app/00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_proto_name_display_null_component() {
        let n = ProtoName::from_strings(["org", "ns", "app"]);
        let s = format!("{}", n);
        assert_eq!(s, "org/ns/app/NULL_COMPONENT");
    }

    #[test]
    fn test_encoded_name_id_when_name_id_none() {
        let enc = EncodedName {
            component_0: 0,
            component_1: 0,
            component_2: 0,
            name_id: None,
        };
        assert_eq!(enc.id(), NameId::NULL_COMPONENT);
        assert_eq!(enc.string_id(), "NULL_COMPONENT");
    }

    #[test]
    fn test_parse_name_valid() {
        let n = ProtoName::parse_name("org/ns/app").unwrap();
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_parse_name_trims_whitespace() {
        let n = ProtoName::parse_name(" org / ns / app ").unwrap();
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_parse_name_empty_string() {
        assert!(ProtoName::parse_name("").is_err());
        assert!(ProtoName::parse_name("   ").is_err());
    }

    #[test]
    fn test_parse_name_too_few_parts() {
        assert!(ProtoName::parse_name("org/ns").is_err());
        assert!(ProtoName::parse_name("org").is_err());
    }

    #[test]
    fn test_parse_name_too_many_parts() {
        assert!(ProtoName::parse_name("a/b/c/d").is_err());
    }

    #[test]
    fn test_parse_name_empty_component() {
        assert!(ProtoName::parse_name("org//app").is_err());
        assert!(ProtoName::parse_name("/ns/app").is_err());
        assert!(ProtoName::parse_name("org/ns/").is_err());
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;

    fn test_subscription_template(
        subscription: bool,
        source: ProtoName,
        dst: ProtoName,
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
        assert_eq!(dst, sub.get_dst());
    }

    fn test_publish_template(
        source: ProtoName,
        dst: ProtoName,
        identity: Option<&str>,
        flags: Option<SlimHeaderFlags>,
    ) {
        let mut builder = ProtoMessage::builder()
            .source(source.clone())
            .destination(dst.clone())
            .application_payload("str", b"this is the content of the message".to_vec());

        if let Some(id) = identity {
            builder = builder.identity(id);
        }

        if let Some(f) = flags.clone() {
            builder = builder.flags(f);
        }

        let pub_msg = builder.build_publish().unwrap();
        let flags = flags.or_else(|| Some(SlimHeaderFlags::default()));

        assert!(pub_msg.is_publish());
        assert!(!pub_msg.is_subscribe());
        assert!(!pub_msg.is_unsubscribe());
        assert_eq!(flags.as_ref().unwrap().recv_from, pub_msg.get_recv_from());
        assert_eq!(flags.as_ref().unwrap().forward_to, pub_msg.get_forward_to());
        assert_eq!(None, pub_msg.try_get_incoming_conn());
        assert_eq!(source, pub_msg.get_source());
        assert_eq!(dst, pub_msg.get_dst());
        assert_eq!(flags.as_ref().unwrap().fanout, pub_msg.get_fanout());
    }

    #[test]
    fn test_subscription() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = ProtoName::from_strings(["org", "ns", "type"]).with_id(2);
        test_subscription_template(true, source.clone(), dst.clone(), None, None);
        test_subscription_template(
            true,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );
        test_subscription_template(
            true,
            source,
            dst,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );
    }

    #[test]
    fn test_unsubscription() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = ProtoName::from_strings(["org", "ns", "type"]).with_id(2);
        test_subscription_template(false, source.clone(), dst.clone(), None, None);
        test_subscription_template(
            false,
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );
        test_subscription_template(
            false,
            source,
            dst,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );
    }

    #[test]
    fn test_publish() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let mut dst = ProtoName::from_strings(["org", "ns", "type"]);
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default()),
        );
        dst.set_id(2);
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default()),
        );
        dst.reset_id();
        test_publish_template(
            source.clone(),
            dst.clone(),
            None,
            Some(SlimHeaderFlags::default().with_recv_from(50)),
        );
        test_publish_template(
            source,
            dst,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(30)),
        );
    }

    #[test]
    fn test_conversions() {
        let name = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
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
        assert_eq!(proto_subscribe.header.as_ref().unwrap().get_dst(), dst);

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

        let proto_publish = ProtoMessage::builder()
            .source(name.clone())
            .destination(dst.clone())
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .application_payload("str", b"this is the content of the message".to_vec())
            .build_publish()
            .unwrap();
        let proto_publish = ProtoPublish::from(proto_publish);
        assert_eq!(proto_publish.header.as_ref().unwrap().get_source(), name);
        assert_eq!(proto_publish.header.as_ref().unwrap().get_dst(), dst);
    }

    #[test]
    fn test_panic() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dst = ProtoName::from_strings(["org", "ns", "type"]).with_id(2);
        let msg = ProtoMessage::builder()
            .source(source)
            .destination(dst)
            .flags(
                SlimHeaderFlags::default()
                    .with_recv_from(2)
                    .with_forward_to(3),
            )
            .build_subscribe()
            .unwrap();

        assert!(std::panic::catch_unwind(|| ProtoUnsubscribe::from(msg.clone())).is_err());
        assert!(std::panic::catch_unwind(|| ProtoPublish::from(msg.clone())).is_err());
        assert!(std::panic::catch_unwind(|| ProtoSubscribe::from(msg)).is_ok());
    }

    #[test]
    fn test_panic_header() {
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
            header_mac: None,
            ttl: DEFAULT_TTL,
            e2e_header_sig: None,
        };

        assert!(std::panic::catch_unwind(|| header.get_source()).is_err());
        assert!(std::panic::catch_unwind(|| header.get_dst()).is_err());
        assert!(std::panic::catch_unwind(|| header.get_recv_from()).is_ok());
        assert!(std::panic::catch_unwind(|| header.get_forward_to()).is_ok());
        assert!(std::panic::catch_unwind(|| header.get_incoming_conn()).is_ok());
        assert!(std::panic::catch_unwind(|| header.get_error()).is_ok());
    }

    #[test]
    fn test_panic_session_header() {
        let header = SessionHeader::new(0, 0, 0, 0);
        assert!(std::panic::catch_unwind(|| header.get_session_id()).is_ok());
        assert!(std::panic::catch_unwind(|| header.get_message_id()).is_ok());
    }

    #[test]
    fn test_panic_proto_message() {
        let message = ProtoMessage {
            metadata: HashMap::new(),
            message_type: None,
        };
        assert!(std::panic::catch_unwind(|| message.get_slim_header()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_type()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_source()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_dst()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_recv_from()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_forward_to()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_incoming_conn()).is_err());
        assert!(std::panic::catch_unwind(|| message.get_fanout()).is_err());
    }

    #[test]
    fn test_service_type_to_int() {
        let total_service_types = SessionMessageType::Ping as i32;
        for i in 0..total_service_types {
            let service_type =
                SessionMessageType::try_from(i).expect("failed to convert int to service type");
            assert_eq!(i32::from(service_type), i32::from(service_type));
        }
        assert!(SessionMessageType::try_from(total_service_types + 1).is_err());
    }

    #[test]
    fn test_proto_message_builder() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "ns", "app"]).with_id(2);

        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .application_payload("test", b"hello world".to_vec())
            .build_publish()
            .unwrap();
        assert!(msg.is_publish());
        assert_eq!(msg.get_source(), source);
        assert_eq!(msg.get_dst(), dest);

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

        let msg = ProtoMessage::builder()
            .source(source.clone())
            .destination(dest.clone())
            .recv_from(10)
            .build_subscribe()
            .unwrap();
        assert!(msg.is_subscribe());
        assert_eq!(msg.get_recv_from(), Some(10));

        let msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
            .forward_to(20)
            .build_unsubscribe()
            .unwrap();
        assert!(msg.is_unsubscribe());
        assert_eq!(msg.get_forward_to(), Some(20));
    }

    #[test]
    fn test_command_payload_builder() {
        let dest = ProtoName::from_strings(["org", "ns", "app"]);

        assert!(CommandPayload::builder()
            .discovery_request()
            .as_discovery_request_payload()
            .is_ok());
        assert!(CommandPayload::builder()
            .discovery_reply()
            .as_discovery_reply_payload()
            .is_ok());

        let payload = CommandPayload::builder().join_request(
            Some(5),
            Some(Duration::from_secs(10)),
            Some(dest.clone()),
            None,
            Some(ProtoMlsSettings::default()),
        );
        let extracted = payload.as_join_request_payload().unwrap();
        assert!(extracted.mls_settings.is_some());
        assert!(extracted.timer_settings.is_some());

        let participant = Participant::new(dest.clone(), ParticipantSettings::bidirectional());
        let payload =
            CommandPayload::builder().join_reply(Some(vec![1, 2, 3]), participant.clone());
        let extracted = payload.as_join_reply_payload().unwrap();
        assert_eq!(extracted.key_package, Some(vec![1, 2, 3]));
        assert_eq!(extracted.participant, Some(participant.clone()));

        assert!(CommandPayload::builder()
            .leave_request()
            .as_leave_request_payload()
            .is_ok());
        assert!(CommandPayload::builder()
            .leave_reply()
            .as_leave_reply_payload()
            .is_ok());

        let participants = vec![participant.clone()];
        let payload =
            CommandPayload::builder().group_add(participant.clone(), participants.clone(), None);
        let extracted = payload.as_group_add_payload().unwrap();
        assert_eq!(extracted.new_participant, Some(participant));
        assert_eq!(extracted.participants, participants);

        let payload =
            CommandPayload::builder().group_remove(dest.clone(), vec![dest.clone()], None);
        assert!(payload
            .as_group_remove_payload()
            .unwrap()
            .removed_participant
            .is_some());
        assert!(!CommandPayload::builder()
            .group_welcome(
                vec![Participant::new(
                    dest.clone(),
                    ParticipantSettings::bidirectional()
                )],
                None
            )
            .as_welcome_payload()
            .unwrap()
            .participants
            .is_empty());
        assert_eq!(
            CommandPayload::builder()
                .group_proposal(Some(dest.clone()), vec![4, 5, 6])
                .as_group_proposal_payload()
                .unwrap()
                .mls_proposal,
            vec![4, 5, 6]
        );
        assert!(CommandPayload::builder()
            .group_ack()
            .as_group_ack_payload()
            .is_ok());
        assert!(CommandPayload::builder()
            .group_nack()
            .as_group_nack_payload()
            .is_ok());
        assert!(CommandPayload::builder().ping().as_ping_payload().is_ok());
    }

    #[test]
    fn test_builder_with_command_payload() {
        let source = ProtoName::from_strings(["org", "ns", "type"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "ns", "app"]).with_id(2);
        let cmd_payload = CommandPayload::builder().discovery_request();

        let msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
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
                link_ecdh_public_key: vec![],
                link_kem_payload: None,
                connection_type: 0,
                node_id: String::new(),
                deployment_name: String::new(),
            })),
        };
        let msg = ProtoMessage::new(HashMap::new(), ProtoLinkMessageType(link));
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_build_link_negotiation_request() {
        let msg = ProtoMessage::builder().build_link_negotiation(
            "my-id",
            "1.2.3",
            false,
            None,
            None,
            LinkConnectionType::Remote,
            "",
            "",
        );
        assert!(msg.is_link());
        assert!(!msg.is_publish());
        assert!(!msg.is_subscribe());
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_build_link_negotiation_reply() {
        let msg = ProtoMessage::builder().build_link_negotiation(
            "my-id",
            "1.2.3",
            true,
            None,
            None,
            LinkConnectionType::Remote,
            "",
            "",
        );
        assert!(msg.is_link());
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_validate_subscribe_missing_source_encoded_name() {
        let valid = ProtoName::from_strings(["org", "ns", "agent"]);
        let hdr = SlimHeader {
            source: Some(ProtoName {
                name: None,
                str_name: None,
            }),
            destination: Some(valid),
            ..Default::default()
        };
        let msg = ProtoMessage::new(
            HashMap::new(),
            ProtoSubscribeType(ProtoSubscribe {
                header: Some(hdr),
                ..Default::default()
            }),
        );
        assert!(matches!(
            msg.validate(),
            Err(MessageError::SourceEncodedNameNotFound)
        ));
    }

    #[test]
    fn test_validate_subscribe_missing_destination_encoded_name() {
        let valid = ProtoName::from_strings(["org", "ns", "agent"]);
        let hdr = SlimHeader {
            source: Some(valid),
            destination: Some(ProtoName {
                name: None,
                str_name: None,
            }),
            ..Default::default()
        };
        let msg = ProtoMessage::new(
            HashMap::new(),
            ProtoSubscribeType(ProtoSubscribe {
                header: Some(hdr),
                ..Default::default()
            }),
        );
        assert!(matches!(
            msg.validate(),
            Err(MessageError::DestinationEncodedNameNotFound)
        ));
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

// ConnType → LinkConnectionType conversion
impl From<slim_config::conn_type::ConnType> for crate::dataplane::proto::v1::LinkConnectionType {
    fn from(ct: slim_config::conn_type::ConnType) -> Self {
        match ct {
            slim_config::conn_type::ConnType::Peer => {
                crate::dataplane::proto::v1::LinkConnectionType::Peer
            }
            slim_config::conn_type::ConnType::Edge => {
                crate::dataplane::proto::v1::LinkConnectionType::Edge
            }
            _ => crate::dataplane::proto::v1::LinkConnectionType::Remote,
        }
    }
}
