// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("local agent not configured")]
    MissingAgentError,
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
    #[error("unknown error")]
    Unknown,
}
