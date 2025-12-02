// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::messages::utils::MessageError;
use slim_config::grpc::errors::ConfigError;
use thiserror::Error;

/// DataPath and subscription table errors merged into a single enum.
#[derive(Error, Debug)]
pub enum DataPathError {
    // Connection lifecycle
    #[error("connection error")]
    ConnectionError,
    #[error("disconnection error")]
    DisconnectionError,

    // Message classification / validation
    #[error("unknown message type")]
    UnknownMsgType,
    #[error("invalid message: {0}")]
    InvalidMessage(MessageError),

    // Subscription / matching
    #[error("no matching found for {0}")]
    NoMatch(String),
    #[error("subscription not found")]
    SubscriptionNotFound,
    #[error("id not found")]
    IdNotFound,

    // Connection lookup
    #[error("connection not found")]
    ConnectionNotFound,
    #[error("connection id not found")]
    ConnectionIdNotFound,

    // Processing
    #[error("message processing error: {0}")]
    ProcessingError(MessageError),

    // Configuration error
    #[error("configuration error: {0}")]
    ConfigurationError(#[from] ConfigError),

    // Shutdown errors
    #[error("data path is already closed")]
    AlreadyClosedError,
    #[error("timeout during shutdown")]
    ShutdownTimeoutError,
}
