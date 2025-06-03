// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum DataPathError {
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("disconnection error: {0}")]
    DisconnectionError(String),
    #[error("unkwon message type {0}")]
    UnknownMsgType(String),
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("unable to set incoming connection")]
    ErrorSettingInConnection(String),
    #[error("error handling subscription: {0}")]
    SubscriptionError(String),
    #[error("error handling unsubscription: {0}")]
    UnsubscriptionError(String),
    #[error("error handling publish: {0}")]
    PublicationError(String),
    #[error("error parsing command message: {0}")]
    CommandError(String),
    #[error("connection not found: {0}")]
    ConnectionNotFound(String),
    #[error("wrong channel type")]
    WrongChannelType,
    #[error("error sending message: {0}")]
    MessageSendError(String),
    #[error("stream error: {0}")]
    StreamError(String),
    #[error("error processing message: {0}")]
    ProcessingError(String),
}
