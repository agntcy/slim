// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use parking_lot::RwLock;

use crate::messages::Agent;

use tracing::error;

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct SubscriptionInfo {
    /// source name of subscription
    source: Agent,

    /// subscription name
    name: Agent,
}

impl SubscriptionInfo {
    pub fn source(&self) -> &Agent {
        &self.source
    }

    pub fn name(&self) -> &Agent {
        &self.name
    }
}

#[derive(Debug, Default)]
pub struct RemoteSubscriptions {
    /// list of subscriptions on a connection (remote)
    /// to create state on the remote host
    table: RwLock<HashMap<u64, HashSet<SubscriptionInfo>>>,
}

impl RemoteSubscriptions {
    pub fn add_subscription(&self, source: Agent, name: Agent, conn: u64) {
        let info = SubscriptionInfo { source, name };
        let mut map = self.table.write();
        match map.get_mut(&conn) {
            None => {
                let mut set = HashSet::new();

                set.insert(info);
                map.insert(conn, set);
            }
            Some(set) => {
                set.insert(info);
            }
        }
    }

    pub fn remove_subscription(&self, source: Agent, name: Agent, conn: u64) {
        let info = SubscriptionInfo { source, name };
        let mut map = self.table.write();
        match map.get_mut(&conn) {
            None => {
                error!("connection not found");
            }
            Some(set) => {
                set.remove(&info);
                if set.is_empty() {
                    map.remove(&conn);
                }
            }
        }
    }

    pub fn get_subscriptions_on_connection(&self, conn: u64) -> HashSet<SubscriptionInfo> {
        let map = self.table.read();
        match map.get(&conn) {
            None => HashSet::new(),
            Some(set) => set.clone(),
        }
    }

    pub fn remove_connection(&self, conn: u64) {
        let mut map = self.table.write();
        map.remove(&conn);
    }
}
