// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;
use crate::messages::utils::MessageError;

/// DataPath and subscription table errors merged into a single enum.
#[derive(Error, Debug, PartialEq)]
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

    // Incoming setup
    #[error("unable to set incoming connection")]
    ErrorSettingInConnection,

    // Subscription operations
    #[error("subscription failure: {0}")]
    SubscriptionError(MessageError),
    #[error("unsubscription failure: {0}")]
    UnsubscriptionError(MessageError),
    #[error("publish failure: {0}")]
    PublicationError(MessageError),
    #[error("no matching found for {0}")]
    NoMatch(String),
    #[error("subscription not found")]
    SubscriptionNotFound,
    #[error("id not found")]
    IdNotFound,

    // Connection lookup
    #[error("connection not found: {connection}")]
    ConnectionNotFound {
        connection: u64,
    },
    #[error("connection id not found")]
    ConnectionIdNotFound,

    // Command parsing
    #[error("command parse error: {0}")]
    CommandError(MessageError),

    // Channel / stream types
    #[error("wrong channel type")]
    WrongChannelType,
    #[error("stream error")]
    StreamError,

    // Channel send failures
    #[error("error sending message to channel: {0}")]
    ChannelSendFailure(String),

    // Generic processing
    #[error("message processing error: {0}")]
    ProcessingError(MessageError),
}
