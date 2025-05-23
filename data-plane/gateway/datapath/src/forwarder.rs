// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::sync::Arc;

use super::tables::connection_table::ConnectionTable;
use super::tables::remote_subscription_table::RemoteSubscriptions;
use super::tables::subscription_table::SubscriptionTableImpl;
use super::tables::{SubscriptionTable, errors::SubscriptionTableError};
use crate::messages::encoder::DEFAULT_AGENT_ID;
use crate::messages::{Agent, AgentType};
use crate::tables::remote_subscription_table::SubscriptionInfo;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Default + Clone,
{
    pub subscription_table: SubscriptionTableImpl,
    remote_subscription_table: RemoteSubscriptions,
    pub connection_table: ConnectionTable<T>,
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
            remote_subscription_table: RemoteSubscriptions::default(),
            connection_table: ConnectionTable::with_capacity(100),
        }
    }

    pub fn on_connection_established(&self, conn: T, existing_index: Option<u64>) -> Option<u64> {
        match existing_index {
            None => {
                let x = self.connection_table.insert(conn) as u64;
                Some(x)
            }
            Some(x) => {
                if self.connection_table.insert_at(conn, x as usize) {
                    existing_index
                } else {
                    None
                }
            }
        }
    }

    pub fn on_connection_drop(&self, conn_index: u64, is_local: bool) {
        self.connection_table.remove(conn_index as usize);
        let _ = self
            .subscription_table
            .remove_connection(conn_index, is_local);
        self.remote_subscription_table.remove_connection(conn_index);
    }

    pub fn get_connection(&self, conn_index: u64) -> Option<Arc<T>> {
        self.connection_table.get(conn_index as usize)
    }

    pub fn get_subscriptions_forwarded_on_connection(
        &self,
        conn_index: u64,
    ) -> HashSet<SubscriptionInfo> {
        self.remote_subscription_table
            .get_subscriptions_on_connection(conn_index)
    }

    pub fn on_subscription_msg(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        conn_index: u64,
        is_local: bool,
        add: bool,
    ) -> Result<(), SubscriptionTableError> {
        if add {
            self.subscription_table
                .add_subscription(agent_type, agent_id, conn_index, is_local)
        } else {
            self.subscription_table
                .remove_subscription(agent_type, agent_id, conn_index, is_local)
        }
    }

    pub fn on_forwarded_subscription(
        &self,
        source: Agent,
        name_type: AgentType,
        name_agent_id: Option<u64>,
        conn_index: u64,
        add: bool,
    ) {
        let name = Agent::new(name_type, name_agent_id.unwrap_or(DEFAULT_AGENT_ID));
        if add {
            self.remote_subscription_table
                .add_subscription(source, name, conn_index);
        } else {
            self.remote_subscription_table
                .remove_subscription(source, name, conn_index);
        }
    }

    pub fn on_publish_msg_match(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        incoming_conn: u64,
        fanout: u32,
    ) -> Result<Vec<u64>, SubscriptionTableError> {
        if fanout == 1 {
            match self
                .subscription_table
                .match_one(agent_type, agent_id, incoming_conn)
            {
                Ok(out) => Ok(vec![out]),
                Err(e) => Err(e),
            }
        } else {
            self.subscription_table
                .match_all(agent_type, agent_id, incoming_conn)
        }
    }

    #[allow(dead_code)]
    pub fn print_subscription_table(&self) -> String {
        format!("{}", self.subscription_table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_forwarder() {
        let agent_class = AgentType::from_strings("Cisco", "Default", "class_ONE");

        let fwd = Forwarder::<u32>::new();

        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), None, 10, false, true),
            Ok(())
        );
        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), Some(1), 12, false, true),
            Ok(())
        );
        assert_eq!(
            // this creates a warning
            fwd.on_subscription_msg(agent_class.clone(), Some(1), 12, false, true),
            Ok(())
        );
        assert_eq!(
            fwd.on_publish_msg_match(agent_class.clone(), Some(1), 100, 1),
            Ok(vec![12])
        );
        assert_eq!(
            fwd.on_publish_msg_match(agent_class.clone(), Some(2), 100, 1),
            Err(SubscriptionTableError::NoMatch(format!(
                "{}, {:?}",
                agent_class,
                Some(2)
            )))
        );

        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), None, 10, false, false),
            Ok(())
        );
        assert_eq!(
            fwd.on_subscription_msg(agent_class.clone(), None, 10, false, false),
            Err(SubscriptionTableError::AgentIdNotFound)
        );
    }
}
