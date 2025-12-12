// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::messages::{Name, utils::MessageError};
use slim_config::grpc::errors::ConfigError;
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

    // Message classification / validation
    #[error("unknown message type")]
    UnknownMsgType,
    #[error("invalid message: {0}")]
    InvalidMessage(MessageError),

    // Subscription / matching
    #[error("no matching found for {0}")]
    NoMatch(Name),
    #[error("subscription not found")]
    SubscriptionNotFound(Name),
    #[error("id not found: {0}")]
    IdNotFound(u64),

    // Connection lookup
    #[error("connection not found: {0}")]
    ConnectionNotFound(u64),
    #[error("connection id not found: {0}")]
    ConnectionIdNotFound(u64),

    // Processing
    #[error("message processing error: {0}")]
    ProcessingError(MessageError),
    #[error("error adding connection to connection table")]
    ConnectionTableAddError,

    // Configuration error
    #[error("configuration error")]
    ConfigurationError(#[from] ConfigError),

    // Shutdown errors
    #[error("data path is already closed")]
    AlreadyClosedError,
    #[error("data pplane is shutting down")]
    ShuttingDownError,
    #[error("timeout during shutdown")]
    ShutdownTimeoutError,
}
