// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

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
    #[error("error creating session: {0}")]
    SessionCreationError(String),
    #[error("error creating the session: {0}")]
    SessionDeletionError(String),
    #[error("error deleting the session: {0}")]
    SessionSendError(String),
    #[error("unknown error")]
    Unknown,
}
