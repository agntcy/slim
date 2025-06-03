// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

use crate::session::SessionMessage;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("agent already registered")]
    AgentAlreadyRegistered,
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("disconnect error: {0}")]
    DisconnectError(String),
    #[error("error sending subscription: {0}")]
    SubscriptionError(String),
    #[error("error sending unsubscription: {0}")]
    UnsubscriptionError(String),
    #[error("error on set route: {0}")]
    SetRouteError(String),
    #[error("error on remove route: {0}")]
    RemoveRouteError(String),
    #[error("error publishing message: {0}")]
    PublishError(String),
    #[error("error receiving message: {0}")]
    ReceiveError(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("error in session: {0}")]
    SessionError(String),
    #[error("client already connected: {0}")]
    ClientAlreadyConnected(String),
    #[error("server not found: {0}")]
    ServerNotFound(String),
    #[error("error sendinfg message: {0}")]
    MessageSendingError(String),
    #[error("unknown error")]
    Unknown,
}

#[derive(Error, Debug, PartialEq)]
pub enum SessionError {
    #[error("error receiving message from slim instance: {0}")]
    SlimReception(String),
    #[error("error sending message to slim instance: {0}")]
    SlimTransmission(String),
    #[error("error in message forwarding: {0}")]
    Forward(String),
    #[error("error receiving message from app: {0}")]
    AppReception(String),
    #[error("error sending message to app: {0}")]
    AppTransmission(String),
    #[error("error processing message: {0}")]
    Processing(String),
    #[error("session id already used: {0}")]
    SessionIdAlreadyUsed(String),
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("missing SLIM header: {0}")]
    MissingSlimHeader(String),
    #[error("missing session header")]
    MissingSessionHeader,
    #[error("session unknown: {0}")]
    SessionUnknown(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("default for session not supported: {0}")]
    SessionDefaultNotSupported(String),
    #[error("missing session id: {0}")]
    MissingSessionId(String),
    #[error("error during message validation: {0}")]
    ValidationError(String),
    #[error("message={message_id} session={session_id}: timeout")]
    Timeout {
        session_id: u32,
        message_id: u32,
        message: Box<SessionMessage>,
    },
    #[error("configuration error: {0}")]
    ConfigurationError(String),
    #[error("message lost: {0}")]
    MessageLost(String),
    #[error("session closed: {0}")]
    SessionClosed(String),
}
