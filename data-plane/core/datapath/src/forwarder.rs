// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::tables::SubscriptionTable;
use super::tables::connection_table::ConnectionTable;
use super::tables::subscription_table::SubscriptionTableImpl;
use super::tables::{ConnType, MatchFilter};
use crate::api::{EncodedName, ProtoName};
use crate::errors::DataPathError;

use tracing::debug;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Clone,
{
    pub subscription_table: SubscriptionTableImpl,
    pub connection_table: ConnectionTable<T>,
}

impl<T> Forwarder<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        Forwarder {
            subscription_table: SubscriptionTableImpl::default(),
            connection_table: ConnectionTable::with_capacity(100),
        }
    }

    pub fn on_connection_established(&self, conn: T, existing_index: Option<u64>) -> Option<u64> {
        match existing_index {
            None => Some(self.connection_table.insert(conn)),
            Some(x) => {
                self.connection_table.insert_at(conn, x);
                existing_index
            }
        }
    }

    pub fn on_connection_drop(
        &self,
        conn_index: u64,
        category: ConnType,
    ) -> HashMap<ProtoName, HashSet<u64>> {
        self.connection_table.remove(conn_index);
        self.subscription_table
            .remove_connection(conn_index, category)
            .unwrap_or_else(|e| {
                debug!(
                    %conn_index, ?category, %e, "failed to remove subscriptions for connection",
                );
                HashMap::new()
            })
    }

    pub fn get_connection(&self, conn_index: u64) -> Option<Arc<T>> {
        self.connection_table.get(conn_index)
    }

    /// Updates the subscription table for the given name/connection.
    pub fn on_subscription_msg(
        &self,
        name: ProtoName,
        conn_index: u64,
        category: ConnType,
        add: bool,
        subscription_id: u64,
    ) -> Result<bool, DataPathError> {
        if add {
            self.subscription_table
                .add_subscription(name, conn_index, category, subscription_id)
        } else {
            self.subscription_table.remove_subscription(
                &name,
                conn_index,
                category,
                subscription_id,
            )
        }
    }

    pub fn on_publish_msg_match(
        &self,
        encoded: EncodedName,
        incoming_conn: u64,
        fanout: u32,
        filter: MatchFilter,
    ) -> Result<Vec<u64>, DataPathError> {
        if fanout == 1 {
            self.subscription_table
                .match_one(&encoded, incoming_conn, filter)
                .map(|out| vec![out])
        } else {
            self.subscription_table
                .match_all(&encoded, incoming_conn, filter)
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

    fn enc(name: &ProtoName) -> EncodedName {
        name.name.unwrap()
    }

    #[test]
    #[traced_test]
    fn test_forwarder() {
        let name = ProtoName::from_strings(["agntcy", "default", "class"]);

        let fwd = Forwarder::<u32>::new();

        assert!(
            fwd.on_subscription_msg(name.clone(), 10, ConnType::Remote, true, 1)
                .is_ok()
        );

        assert!(
            fwd.on_subscription_msg(name.clone().with_id(1), 12, ConnType::Remote, true, 2)
                .is_ok()
        );

        assert!(
            // this creates a warning
            fwd.on_subscription_msg(name.clone().with_id(1), 12, ConnType::Remote, true, 3)
                .is_ok()
        );

        assert_eq!(
            fwd.on_publish_msg_match(enc(&name.clone().with_id(1)), 100, 1, MatchFilter::ALL)
                .unwrap(),
            vec![12]
        );

        let expected = name.clone().with_id(2);

        let err = fwd.on_publish_msg_match(enc(&expected), 100, 1, MatchFilter::ALL);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(..))));

        assert!(
            fwd.on_subscription_msg(name.clone(), 10, ConnType::Remote, false, 1)
                .is_ok()
        );

        let err = fwd.on_subscription_msg(name.clone(), 10, ConnType::Remote, false, 1);
        assert!(matches!(err, Err(DataPathError::IdNotFound(_))));
    }
}
