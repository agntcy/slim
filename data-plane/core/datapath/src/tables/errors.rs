// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum SubscriptionTableError {
    #[error("no matching found for {0}")]
    NoMatch(String),
    #[error("subscription not found")]
    SubscriptionNotFound,
    #[error("id not found")]
    IdNotFound,
    #[error("connection id not found")]
    ConnectionIdNotFound,
}
