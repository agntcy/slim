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

        // Generic processing
        #[error("message processing error: {0}")]
        ProcessingError(MessageError),
}
