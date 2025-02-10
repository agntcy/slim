// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum SubscriptionTableError {
    #[error("no matching found")]
    MatchNotFound,
    #[error("subscription already exists")]
    SubscriptionExists,
    #[error("subscription not fund")]
    SubscriptionNotFound,
    #[error("agent id not fund")]
    AgentIdNotFound,
    #[error("connection id not fund")]
    ConnectionIdNotFound,
    #[error("connection already exists")]
    ConnectionExists,
}