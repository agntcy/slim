// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

use crate::session::SessionMessage;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("agent already registered: {0}")]
    AgentAlreadyRegistered(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("disconnect error")]
    DisconnectError,
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
    #[error("error in session session: {0}")]
    SessionError(String),
    #[error("unknown error")]
    Unknown,
}

#[derive(Error, Debug, PartialEq)]
pub enum SessionError {
    #[error("error receiving message from gateway: {0}")]
    GatewayReception(String),
    #[error("error sending message to gateway: {0}")]
    GatewayTransmission(String),
    #[error("error receiving message from app: {0}")]
    AppReception(String),
    #[error("error sending message to app: {0}")]
    AppTransmission(String),
    #[error("error processing message: {0}")]
    Processing(String),
    #[error("session id already used: {0}")]
    SessionIdAlreadyUsed(String),
    #[error("missing AGP header: {0}")]
    MissingAgpHeader(String),
    #[error("missing session header")]
    MissingSessionHeader,
    #[error("session unknown: {0}")]
    SessionUnknown(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("missing session id: {0}")]
    MissingSessionId(String),
    #[error("timeout for message: {error}")]
    Timeout {
        error: String,
        message: Box<SessionMessage>,
    },
}
