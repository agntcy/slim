// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum SubscriptionTableError {
    #[error("no matching found")]
    NoMatch,
    #[error("subscription not fund")]
    SubscriptionNotFound,
    #[error("agent id not fund")]
    AgentIdNotFound,
    #[error("connection id not fund")]
    ConnectionIdNotFound,
}
