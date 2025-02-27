use std::collections::{HashMap, HashSet};

use parking_lot::RwLock;

use crate::messages::Agent;

use tracing::error;

#[derive(Debug, Default)]
pub struct RemoteSubscriptions {
    /// list of subscriptions on a connection (remote)
    /// to create state on the remote host
    table: RwLock<HashMap<u64, HashSet<Agent>>>,
}

impl RemoteSubscriptions {
    pub fn add_subscription(&self, subscription: Agent, conn: u64) {
        let mut map = self.table.write();
        match map.get_mut(&conn) {
            None => {
                let mut set = HashSet::new();
                set.insert(subscription);
                map.insert(conn, set);
            }
            Some(set) => {
                set.insert(subscription);
            }
        }
    }

    pub fn remove_subscription(&self, subscription: Agent, conn: u64) {
        let mut map = self.table.write();
        match map.get_mut(&conn) {
            None => {
                error!("connection not found");
            }
            Some(set) => {
                set.remove(&subscription);
                if set.is_empty() {
                    map.remove(&conn);
                }
            }
        }
    }

    pub fn get_subscriptions_on_connection(&self, conn: u64) -> HashSet<Agent> {
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
