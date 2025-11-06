// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

pub mod connection_table;
pub mod errors;
pub mod remote_subscription_table;
pub mod subscription_table;

pub mod pool;

use crate::messages::Name;
use errors::SubscriptionTableError;

pub trait SubscriptionTable {
    fn for_each<F>(&self, f: F)
    where
        F: FnMut(&Name, u64, &[u64], &[u64]);

    fn add_subscription(
        &self,
        name: Name,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_subscription(
        &self,
        name: &Name,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError>;

    fn remove_connection(
        &self,
        conn: u64,
        is_local: bool,
    ) -> Result<HashSet<Name>, SubscriptionTableError>;

    fn match_one(&self, name: &Name, incoming_conn: u64) -> Result<u64, SubscriptionTableError>;

    fn match_all(
        &self,
        name: &Name,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError>;
}
