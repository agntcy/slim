// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub mod connection_table;
pub mod errors;
pub mod subscription_table;

mod pool;

use crate::messages::{Agent, AgentClass};
use errors::SubscriptionTableError;
use std::collections::HashSet;

pub trait SubscriptionTable {
    const DEFAULT_AGENT_ID: u64 = 0;

    fn add_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn get_subscriptions_on_connection(&self, conn: u64) -> HashSet<Agent>;

    fn remove_connection(&self, conn: u64, is_local: bool) -> Result<(), SubscriptionTableError>;

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
