// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::tables::SubscriptionTable;
use super::tables::connection_table::ConnectionTable;
use super::tables::subscription_table::SubscriptionTableImpl;
use super::tables::{ConnType, MatchFilter};
use crate::api::{EncodedName, ProtoName};
use crate::connection::Connection;
use crate::errors::DataPathError;
use crate::tables::default_gateway::{DefaultGateway, Gateway};
use tracing::debug;

#[derive(Debug)]
pub struct Forwarder<T>
where
    T: Clone,
{
    pub subscription_table: SubscriptionTableImpl,
    pub connection_table: ConnectionTable<T>,
    default_gateway: DefaultGateway,
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
            default_gateway: DefaultGateway::new(),
        }
    }

    pub fn on_connection_established(&self, conn: T, existing_index: Option<u64>) -> Option<u64> {
        let id = match existing_index {
            None => Some(self.connection_table.insert(conn)),
            Some(x) => {
                self.connection_table.insert_at(conn, x);
                existing_index
            }
        };
        self.default_gateway.invalidate();
        id
    }

    pub fn on_connection_drop(
        &self,
        conn_index: u64,
        category: ConnType,
    ) -> HashMap<ProtoName, HashSet<u64>> {
        self.connection_table.remove(conn_index);
        self.default_gateway.invalidate();
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

    pub fn on_connection_type_changed(&self) {
        self.default_gateway.invalidate();
    }

    pub fn set_default_gateway_enabled(&self, enabled: bool) {
        self.default_gateway.set_enabled(enabled);
    }

    pub fn clear_default_gateway(&self) {
        self.default_gateway.set_explicit(None);
        self.default_gateway.invalidate();
    }

    pub fn default_gateway_state(&self) -> Gateway {
        self.default_gateway.cached()
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

impl Forwarder<Connection> {
    fn edge_conn_ids(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.connection_table.for_each(|id, conn| {
            if conn.connection_type() == ConnType::Edge {
                out.push(id);
            }
        });
        out
    }

    /// Pin an Edge connection as the default gateway. Rejects missing or non-Edge ids.
    pub fn set_default_gateway(&self, conn_id: u64) -> Result<(), DataPathError> {
        let conn = self.connection_table.get(conn_id).ok_or_else(|| {
            DataPathError::InvalidDefaultGateway {
                conn_id,
                reason: "connection not found".to_string(),
            }
        })?;

        if conn.connection_type() != ConnType::Edge {
            return Err(DataPathError::InvalidDefaultGateway {
                conn_id,
                reason: format!(
                    "connection type is {:?}, expected Edge",
                    conn.connection_type()
                ),
            });
        }
        self.default_gateway.set_explicit(Some(conn_id));
        Ok(())
    }

    pub fn resolve_default_gateway(&self) -> Gateway {
        self.default_gateway.resolve(|| self.edge_conn_ids())
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

    fn dummy_conn(ty: ConnType) -> Connection {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        Connection::new(ty, crate::connection::Channel::Server(tx))
    }

    #[test]
    fn resolve_auto_picks_single_edge() {
        let fwd = Forwarder::<Connection>::new();
        let local = fwd
            .on_connection_established(dummy_conn(ConnType::Local), None)
            .unwrap();
        let edge = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        assert_ne!(local, edge);
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Some {
                conn_id: edge,
                source: crate::tables::default_gateway::GatewaySource::Auto,
            }
        )
    }

    #[test]
    fn set_default_gateway_rejects_non_edge() {
        let fwd = Forwarder::<Connection>::new();
        let peer = fwd
            .on_connection_established(dummy_conn(ConnType::Peer), None)
            .unwrap();
        let err = fwd.set_default_gateway(peer).unwrap_err();
        assert!(matches!(
            err,
            DataPathError::InvalidDefaultGateway { conn_id, .. } if conn_id == peer
        ));
        let err = fwd.set_default_gateway(999).unwrap_err();
        assert!(matches!(
            err,
            DataPathError::InvalidDefaultGateway { conn_id: 999, .. }
        ));
    }

    #[test]
    fn set_default_gateway_pins_edge_among_two() {
        let fwd = Forwarder::<Connection>::new();
        let a = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        let b = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Ambiguous { count: 2 }
        );
        fwd.set_default_gateway(a).unwrap();
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Some {
                conn_id: a,
                source: crate::tables::default_gateway::GatewaySource::Explicit,
            }
        );
        assert_ne!(a, b);
    }

    #[test]
    fn establish_and_drop_invalidate_auto_cache() {
        let fwd = Forwarder::<Connection>::new();
        let first = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        assert!(matches!(
            fwd.resolve_default_gateway(),
            Gateway::Some { conn_id, .. } if conn_id == first
        ));
        let _second = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Ambiguous { count: 2 }
        );
        fwd.on_connection_drop(_second, ConnType::Edge);
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Some {
                conn_id: first,
                source: crate::tables::default_gateway::GatewaySource::Auto,
            }
        );
    }

    #[test]
    fn explicit_gateway_does_not_fall_back_after_drop() {
        let fwd = Forwarder::<Connection>::new();
        let pinned = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        let remaining = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();

        fwd.set_default_gateway(pinned).unwrap();
        fwd.on_connection_drop(pinned, ConnType::Edge);

        assert_eq!(fwd.resolve_default_gateway(), Gateway::Empty);
        assert_ne!(pinned, remaining);
    }

    #[test]
    fn disabling_auto_still_allows_explicit_gateway() {
        let fwd = Forwarder::<Connection>::new();
        let edge = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();

        fwd.set_default_gateway_enabled(false);
        assert_eq!(fwd.resolve_default_gateway(), Gateway::Empty);

        fwd.set_default_gateway(edge).unwrap();
        assert_eq!(
            fwd.resolve_default_gateway(),
            Gateway::Some {
                conn_id: edge,
                source: crate::tables::default_gateway::GatewaySource::Explicit,
            }
        );
    }

    #[test]
    fn type_change_invalidates_auto_cache() {
        let fwd = Forwarder::<Connection>::new();
        let edge = fwd
            .on_connection_established(dummy_conn(ConnType::Edge), None)
            .unwrap();
        assert!(matches!(
            fwd.resolve_default_gateway(),
            Gateway::Some { conn_id, .. } if conn_id == edge
        ));

        fwd.connection_table
            .update(edge, |conn| conn.set_connection_type(ConnType::Peer));
        fwd.on_connection_type_changed();

        assert_eq!(fwd.resolve_default_gateway(), Gateway::Empty);
    }
}
