// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub mod connection_table;
pub mod pool;
pub mod subscription_table;

use gateway_messages::encoder::AgentClass;
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

pub trait SubscriptionTable {
    const DEFAULT_AGENT_ID: u64 = 0;

    fn add_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_connection(&self, conn: u64) -> Result<(), SubscriptionTableError>;

    fn match_one(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<u64, SubscriptionTableError>;
    fn match_all(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError>;
}
