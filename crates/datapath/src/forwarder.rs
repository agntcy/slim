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
use crate::logical_connection::{LogicalConnectionTable, SubConnPolicy};

use tracing::debug;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Clone,
{
    pub subscription_table: SubscriptionTableImpl,
    pub connection_table: ConnectionTable<T>,
    /// Groups physical connections that terminate on the same remote node.
    ///
    /// The subscription table is keyed on the ids this produces, not on physical
    /// connection ids — see [`crate::logical_connection`].
    pub logical_connections: LogicalConnectionTable,
}

impl<T> Default for Forwarder<T>
where
    T: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Forwarder<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        Forwarder {
            subscription_table: SubscriptionTableImpl::default(),
            connection_table: ConnectionTable::with_capacity(100),
            logical_connections: LogicalConnectionTable::new(),
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

    /// Group a physical connection under the logical connection for `group_key`.
    ///
    /// Returns the logical id that routing state for this connection must be
    /// registered under.
    pub fn attach_sub_connection(
        &self,
        conn_index: u64,
        group_key: &str,
        policy: SubConnPolicy,
    ) -> u64 {
        self.logical_connections
            .attach(conn_index, group_key, policy)
    }

    /// The routing id for `conn_index` — its logical connection when grouped,
    /// otherwise `conn_index` itself. Idempotent.
    pub fn routing_id(&self, conn_index: u64) -> u64 {
        self.logical_connections.resolve(conn_index)
    }

    /// The physical connections a message addressed to `id` should be written to,
    /// after applying the logical connection's sub-connection policy.
    pub fn select_sub_connections(&self, id: u64, affinity_key: Option<u64>) -> Vec<u64> {
        self.logical_connections.select(id, affinity_key)
    }

    /// Tear down state for a dropped physical connection.
    ///
    /// Subscriptions are keyed on the logical connection, so they are only removed
    /// when the last sub-connection goes away. While any sibling sub-connection
    /// survives, the routing entry stays valid and traffic simply moves over —
    /// which is why this returns an empty map in that case, suppressing the
    /// unsubscribe notifications the caller would otherwise send.
    pub fn on_connection_drop(
        &self,
        conn_index: u64,
        category: ConnType,
    ) -> HashMap<ProtoName, HashSet<u64>> {
        self.connection_table.remove(conn_index);

        let outcome = self.logical_connections.detach(conn_index);
        if !outcome.was_last {
            debug!(
                %conn_index,
                logical_id = %outcome.logical_id,
                ?category,
                "sub-connection dropped but logical connection survives; keeping subscriptions",
            );
            return HashMap::new();
        }

        self.subscription_table
            .remove_connection(outcome.logical_id, category)
            .unwrap_or_else(|e| {
                debug!(
                    %conn_index,
                    logical_id = %outcome.logical_id,
                    ?category, %e,
                    "failed to remove subscriptions for connection",
                );
                HashMap::new()
            })
    }

    pub fn get_connection(&self, conn_index: u64) -> Option<Arc<T>> {
        self.connection_table.get(conn_index)
    }

    /// Updates the subscription table for the given name/connection.
    ///
    /// `conn_index` may be either physical or logical; it is canonicalised to the
    /// logical id before touching the table, so a subscription arriving on any
    /// sub-connection of a peer registers exactly one entry for that peer.
    pub fn on_subscription_msg(
        &self,
        name: ProtoName,
        conn_index: u64,
        category: ConnType,
        add: bool,
        subscription_id: u64,
    ) -> Result<bool, DataPathError> {
        let conn = self.routing_id(conn_index);
        if add {
            self.subscription_table
                .add_subscription(name, conn, category, subscription_id)
        } else {
            self.subscription_table
                .remove_subscription(&name, conn, category, subscription_id)
        }
    }

    /// Match a publish destination, returning routing ids (logical where the peer
    /// has a logical connection, physical otherwise).
    ///
    /// `incoming_conn` is canonicalised first, which is what makes the exclusion
    /// cover every sub-connection of the peer the message came from rather than
    /// just the one socket it arrived on.
    pub fn on_publish_msg_match(
        &self,
        encoded: EncodedName,
        incoming_conn: u64,
        fanout: u32,
        filter: MatchFilter,
    ) -> Result<Vec<u64>, DataPathError> {
        let incoming = self.routing_id(incoming_conn);
        if fanout == 1 {
            self.subscription_table
                .match_one(&encoded, incoming, filter)
                .map(|out| vec![out])
        } else {
            self.subscription_table
                .match_all(&encoded, incoming, filter)
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
