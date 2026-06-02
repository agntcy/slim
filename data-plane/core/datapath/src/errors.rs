// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::string;

use crate::api::ProtoName;
use crate::api::ProtoSessionMessageType;
use crate::api::proto::dataplane::v1::Message;
use crate::messages::utils::MessageError;
use slim_config::errors::ConfigError;
use thiserror::Error;

/// DataPath and subscription table errors merged into a single enum.
#[derive(Error, Debug)]
pub enum DataPathError {
    // Connection lifecycle
    #[error("connection error")]
    ConnectionError,
    #[error("disconnection error")]
    DisconnectionError(u64),
    #[error("grpc error")]
    GrpcError(#[from] tonic::Status),
    #[error("link negotiation error")]
    NegotiationError(String),

    // Message classification / validation
    #[error("unknown message type")]
    UnknownMsgType,
    #[error("invalid message: {0}")]
    InvalidMessage(MessageError),
    #[error("invalid name id format: {0}")]
    InvalidNameIdFormat(String),
    #[error("invalid name format: {0}")]
    InvalidNameFormat(String),

    // Subscription / matching
    #[error("no matching found for [{:x}, {:x}, {:x}, {}]", .0, .1, .2, .3)]
    NoMatchEncoded(u64, u64, u64, String),
    #[error("subscription not found")]
    SubscriptionNotFound(ProtoName),
    #[error("subscription id not found: {0}")]
    SubscriptionIdNotFound(u64),
    #[error("id not found: {0}")]
    IdNotFound(string::String),

    // Connection lookup
    #[error("connection not found: {0}")]
    ConnectionNotFound(u64),
    #[error("connection id not found: {0}")]
    ConnectionIdNotFound(u64),

    // Processing
    #[error("malformed message")]
    MalformedMessage(#[from] MessageError),
    #[error("message processing error: {0}")]
    ProcessingError(MessageError),
    #[error("error adding connection to connection table")]
    ConnectionTableAddError,
    #[error("message processing error: {source}")]
    MessageProcessingError {
        #[source]
        source: Box<DataPathError>,
        msg: Box<Message>,
    },

    // Configuration error
    #[error("configuration error")]
    ConfigurationError(#[from] ConfigError),

    // Remote subscription ACK errors
    #[error("remote subscription ack timed out after {0} retries")]
    RemoteSubscriptionAckTimeout(u32),

    #[error("remote subscription ack returned error: {0}")]
    RemoteSubscriptionAckError(String),

    // Shutdown errors
    #[error("data path is already closed")]
    AlreadyClosedError,
    #[error("data plane is shutting down")]
    ShuttingDownError,
    #[error("timeout during shutdown")]
    ShutdownTimeoutError,

    #[error("SLIM header integrity: {0}")]
    HeaderIntegrity(#[from] crate::header_mac::HeaderMacError),

    #[error("header MAC requires completed link negotiation on connection {0}")]
    HeaderMacAwaitingLinkNegotiation(u64),

    #[error("inter-node ephemeral key generation failed")]
    LinkKeyGeneration,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageContext {
    pub message_id: u32,
    pub session_id: u32,
    pub session_message_type: i32,
}

impl MessageContext {
    pub fn from_msg(msg: &Message) -> Option<Self> {
        msg.try_get_session_header().map(|header| Self {
            message_id: header.get_message_id(),
            session_id: header.get_session_id(),
            session_message_type: header.session_message_type().into(),
        })
    }

    pub fn get_session_message_type(&self) -> ProtoSessionMessageType {
        self.session_message_type
            .try_into()
            .unwrap_or(ProtoSessionMessageType::Unspecified)
    }
}

/// A unified error payload that includes an error message and optional session context.
/// This type is used to serialize/deserialize errors sent over gRPC with consistent JSON structure.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorPayload {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_context: Option<MessageContext>,
}

impl std::fmt::Display for ErrorPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ErrorPayload: {}", self.error)?;
        match &self.session_context {
            Some(ctx) => write!(
                f,
                " (session_id={}, message_id={}, session_message_type={:?})",
                ctx.session_id,
                ctx.message_id,
                ctx.get_session_message_type()
            ),
            None => Ok(()),
        }
    }
}

impl ErrorPayload {
    /// Create a new error payload
    pub fn new(error: String, session_context: Option<MessageContext>) -> Self {
        Self {
            error,
            session_context,
        }
    }

    /// Convert to JSON string for transmission
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).expect("ErrorPayload should be serializable")
    }

    /// Parse from JSON string
    pub fn from_json_str(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}
