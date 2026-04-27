// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::tables::SubscriptionTable;
use super::tables::connection_table::ConnectionTable;
use super::tables::remote_subscription_table::RemoteSubscriptions;
use super::tables::subscription_table::SubscriptionTableImpl;
use crate::api::EncodedName;
use crate::errors::DataPathError;
use crate::messages::Name;
use crate::tables::remote_subscription_table::SubscriptionInfo;

use tracing::debug;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Clone,
{
    pub subscription_table: SubscriptionTableImpl,
    remote_subscription_table: RemoteSubscriptions,
    pub connection_table: ConnectionTable<T>,
}

impl<T> Forwarder<T>
where
    T: Clone,
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
        is_local: bool,
    ) -> (HashMap<Name, HashSet<u64>>, HashSet<SubscriptionInfo>) {
        self.connection_table.remove(conn_index);
        let local_subs = self
            .subscription_table
            .remove_connection(conn_index, is_local)
            .unwrap_or_else(|e| {
                debug!(
                    %conn_index, %is_local, %e, "failed to remove local subscriptions for connection",
                );
                HashMap::new()
            });
        let remote_subs = self.remote_subscription_table.remove_connection(conn_index);
        (local_subs, remote_subs)
    }

    pub fn get_connection(&self, conn_index: u64) -> Option<Arc<T>> {
        self.connection_table.get(conn_index)
    }

    pub fn get_subscriptions_forwarded_on_connection(
        &self,
        conn_index: u64,
    ) -> HashSet<SubscriptionInfo> {
        self.remote_subscription_table
            .get_subscriptions_on_connection(conn_index)
    }

    /// Updates the subscription table for the given name/connection.
    pub fn on_subscription_msg(
        &self,
        name: Name,
        conn_index: u64,
        is_local: bool,
        add: bool,
        subscription_id: u64,
    ) -> Result<(), DataPathError> {
        if add {
            self.subscription_table
                .add_subscription(name, conn_index, is_local, subscription_id)
        } else {
            self.subscription_table.remove_subscription(
                &name,
                conn_index,
                is_local,
                subscription_id,
            )
        }
    }

    pub fn on_forwarded_subscription(
        &self,
        source: Name,
        name: Name,
        source_identity: String,
        conn_index: u64,
        add: bool,
        subscription_id: u64,
    ) {
        if add {
            self.remote_subscription_table.add_subscription(
                source,
                name,
                source_identity,
                conn_index,
                subscription_id,
            );
        } else {
            self.remote_subscription_table.remove_subscription(
                source,
                name,
                source_identity,
                conn_index,
                subscription_id,
            );
        }
    }

    pub fn on_publish_msg_match(
        &self,
        encoded: EncodedName,
        incoming_conn: u64,
        fanout: u32,
    ) -> Result<Vec<u64>, DataPathError> {
        if fanout == 1 {
            self.subscription_table
                .match_one(&encoded, incoming_conn)
                .map(|out| vec![out])
        } else {
            self.subscription_table.match_all(&encoded, incoming_conn)
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
        let name = Name::from_strings(["agntcy", "default", "class"]);

        let fwd = Forwarder::<u32>::new();

        assert!(
            fwd.on_subscription_msg(name.clone(), 10, false, true, 1)
                .is_ok()
        );

        assert!(
            fwd.on_subscription_msg(name.clone().with_id(1), 12, false, true, 2)
                .is_ok()
        );

        assert!(
            // this creates a warning
            fwd.on_subscription_msg(name.clone().with_id(1), 12, false, true, 3)
                .is_ok()
        );

        assert_eq!(
            fwd.on_publish_msg_match(EncodedName::from(&name.clone().with_id(1)), 100, 1)
                .unwrap(),
            vec![12]
        );

        let expected = name.clone().with_id(2);

        let err = fwd.on_publish_msg_match(EncodedName::from(&expected), 100, 1);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(_))));

        assert!(
            fwd.on_subscription_msg(name.clone(), 10, false, false, 1)
                .is_ok()
        );

        let err = fwd.on_subscription_msg(name.clone(), 10, false, false, 1);
        assert!(matches!(err, Err(DataPathError::IdNotFound(_))));
    }
}
