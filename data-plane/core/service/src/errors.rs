// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::errors::AuthError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("auth error: {0}")]
    AuthError(#[from] AuthError),
    #[error("app already registered")]
    AppAlreadyRegistered,
    #[error("app not found: {0}")]
    AppNotFound(String),
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
    #[error("to be able to call invite/remove, session must be multicast: {0}")]
    SessionMustBeMulticast(String),
    #[error("error in session: {0}")]
    SessionError(String),
    #[error("client already connected: {0}")]
    ClientAlreadyConnected(String),
    #[error("server not found: {0}")]
    ServerNotFound(String),
    #[error("error sending message: {0}")]
    MessageSendingError(String),
    #[error("error in controller: {0}")]
    ControllerError(String),
    #[error("storage error: {0}")]
    StorageError(String),
    #[error("drain signal missing")]
    NoDrainSignal,
    #[error("timed out while waiting for sessions to close")]
    DrainTimeoutError,
    #[error("unknown error")]
    Unknown,
}
