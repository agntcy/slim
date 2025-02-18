// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use super::tables::connection_table::ConnectionTable;
use super::tables::subscription_table::SubscriptionTableImpl;
use super::tables::{errors::SubscriptionTableError, SubscriptionTable};
use crate::messages::AgentClass;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Default + Clone,
{
    subscription_table: SubscriptionTableImpl,
    connection_table: ConnectionTable<T>,
}

impl<T> Default for Forwarder<T>
where
    T: Default + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Forwarder<T>
where
    T: Default + Clone,
{
    pub fn new() -> Self {
        Forwarder {
            subscription_table: SubscriptionTableImpl::default(),
            connection_table: ConnectionTable::with_capacity(100),
        }
    }

    pub fn on_connection_established(&self, conn: T) -> u64 {
        self.connection_table.insert(conn) as u64
    }

    pub fn on_connection_drop(&self, conn_index: u64, is_local: bool) {
        self.connection_table.remove(conn_index as usize);
        let _ = self
            .subscription_table
            .remove_connection(conn_index, is_local);
    }

    pub fn get_connection(&self, conn_index: u64) -> Option<Arc<T>> {
        self.connection_table.get(conn_index as usize)
    }

    pub fn on_subscription_msg(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn_index: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        self.subscription_table
            .add_subscription(class, agent_id, conn_index, is_local)
    }

    pub fn on_unsubscription_msg(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn_index: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        self.subscription_table
            .remove_subscription(class, agent_id, conn_index, is_local)
    }

    pub fn on_publish_msg_match_one(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<u64, SubscriptionTableError> {
        self.subscription_table
            .match_one(class, agent_id, incoming_conn)
    }

    pub fn on_publish_msg_match_all(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError> {
        self.subscription_table
            .match_all(class, agent_id, incoming_conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::encoder::encode_agent_class;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_forwarder() {
        let agent_class = encode_agent_class("Cisco", "Default", "class_ONE");

        let fwd = Forwarder::<u32>::new();

        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), None, 10, false),
            Ok(())
        );
        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), Some(1), 12, false),
            Ok(())
        );
        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), Some(1), 12, false),
            Err(SubscriptionTableError::SubscriptionExists)
        );
        assert_eq!(
            fwd.on_publish_msg_match_one(agent_class.clone(), Some(1), 100),
            Ok(12)
        );
        assert_eq!(
            fwd.on_publish_msg_match_one(agent_class.clone(), Some(2), 100),
            Err(SubscriptionTableError::MatchNotFound)
        );

        assert_eq!(
            fwd.on_unsubscription_msg(agent_class.clone(), None, 10, false),
            Ok(())
        );
        assert_eq!(
            fwd.on_unsubscription_msg(agent_class.clone(), None, 10, false),
            Err(SubscriptionTableError::AgentIdNotFound)
        );
    }
}
