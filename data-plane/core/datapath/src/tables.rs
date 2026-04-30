// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

pub mod connection_table;
pub mod remote_subscription_table;
pub mod subscription_table;

pub mod pool;

use crate::messages::Name;

pub trait SubscriptionTable {
    type Error;

    fn for_each<F>(&self, f: F)
    where
        F: FnMut(&Name, u64, &[u64], &[u64]);

    fn add_subscription(
        &self,
        name: Name,
        conn: u64,
        is_local: bool,
        subscription_id: u64,
    ) -> Result<(), Self::Error>;

    fn remove_subscription(
        &self,
        name: &Name,
        conn: u64,
        is_local: bool,
        subscription_id: u64,
    ) -> Result<(), Self::Error>;

    /// Remove all subscriptions for `conn` and return a map of each name to its set of
    /// subscription IDs, so that callers can restore the exact state later if needed.
    fn remove_connection(
        &self,
        conn: u64,
        is_local: bool,
    ) -> Result<HashMap<Name, HashSet<u64>>, Self::Error>;

    fn match_one(&self, name: &Name, incoming_conn: u64) -> Result<u64, Self::Error>;

    fn match_all(&self, name: &Name, incoming_conn: u64) -> Result<Vec<u64>, Self::Error>;
}
