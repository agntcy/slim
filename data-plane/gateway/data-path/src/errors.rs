// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;
use tokio::sync::mpsc;
use tonic::Status;

use agp_gw_pubsub_proto::proto::pubsub::v1::Message;

#[derive(Debug, Clone)]
pub enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
    Unkwon,
}

#[derive(Error, Debug, PartialEq)]
pub enum DataPathError {
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("unkwon message type {0}")]
    UnknownMsgType(String),
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
}
