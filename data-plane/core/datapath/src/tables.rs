// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod connection_table;
pub mod errors;
pub mod remote_subscription_table;
pub mod subscription_table;

pub mod pool;

use crate::messages::AgentType;
use errors::SubscriptionTableError;

pub trait SubscriptionTable {
    fn for_each<F>(&self, f: F)
    where
        F: FnMut(&AgentType, u64, &[u64], &[u64]);

    fn add_subscription(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_subscription(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_connection(&self, conn: u64, is_local: bool) -> Result<(), SubscriptionTableError>;

    fn match_one(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<u64, SubscriptionTableError>;

    fn match_all(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError>;
}
