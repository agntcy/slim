// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Logical connections: one entry per remote SLIM node, many physical sub-connections.
//!
//! # Why
//!
//! The subscription table's loop prevention is a scalar comparison against the
//! connection a message arrived on. When several physical connections terminate on
//! the *same* remote node — which happens whenever more than one local application
//! dials the same peer — the table cannot tell them apart from connections to
//! *different* nodes, so it happily returns a sibling connection as a forwarding
//! target and the message goes straight back where it came from.
//!
//! A logical connection is the missing indirection: physical connections that share
//! a remote identity are grouped under one id, the routing tables are keyed on that
//! id, and excluding "the connection this arrived on" excludes the whole peer.
//!
//! # Identity space
//!
//! Physical connection ids are dense indices into
//! [`Pool`](crate::tables::pool::Pool), so they start at 0 and stay small. Logical
//! ids are allocated from [`LOGICAL_ID_BASE`] upwards, which keeps the two spaces
//! disjoint and lets a single `u64` be interpreted unambiguously via
//! [`is_logical`]. Everything that stores or compares connection ids for *routing*
//! (subscription table, `match_*` exclusion) uses the logical id; everything that
//! touches a socket uses the physical id.
//!
//! Connections that are not grouped — local application connections, and remote
//! connections whose peer did not advertise an identity — resolve to their own
//! physical id. That is what makes single-connection deployments behave exactly as
//! they did before: the logical id *is* the physical id, and no lookup changes.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tracing::debug;

pub use slim_config::sub_conn::SubConnPolicy;

/// First logical connection id. Physical ids are `Pool` indices and never reach
/// this value, so the two id spaces cannot collide.
pub const LOGICAL_ID_BASE: u64 = 1 << 63;

/// Whether `id` denotes a logical connection rather than a physical one.
pub const fn is_logical(id: u64) -> bool {
    id >= LOGICAL_ID_BASE
}

/// A logical connection: the set of physical connections reaching one remote node.
#[derive(Debug)]
struct LogicalConn {
    /// Grouping key — the remote's advertised identity, or an explicit override.
    key: String,
    /// Physical connection ids, in attach order. The first entry is the primary
    /// for [`SubConnPolicy::Failover`].
    sub_conns: Vec<u64>,
    /// How to pick among `sub_conns`.
    policy: SubConnPolicy,
    /// Cursor for round-robin selection.
    cursor: AtomicUsize,
}

impl Clone for LogicalConn {
    fn clone(&self) -> Self {
        LogicalConn {
            key: self.key.clone(),
            sub_conns: self.sub_conns.clone(),
            policy: self.policy,
            cursor: AtomicUsize::new(self.cursor.load(Ordering::Relaxed)),
        }
    }
}

impl LogicalConn {
    /// Pick the sub-connections to send a message on.
    ///
    /// `affinity_key` is only consulted by [`SubConnPolicy::Affinity`]; callers that
    /// have no flow identity to offer pass `None`, which degrades to round-robin.
    fn select(&self, affinity_key: Option<u64>) -> Vec<u64> {
        let n = self.sub_conns.len();
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![self.sub_conns[0]];
        }

        match self.policy {
            SubConnPolicy::Redundant => self.sub_conns.clone(),
            SubConnPolicy::Failover => vec![self.sub_conns[0]],
            SubConnPolicy::Affinity => match affinity_key {
                Some(k) => vec![self.sub_conns[(k % n as u64) as usize]],
                None => vec![self.round_robin(n)],
            },
            SubConnPolicy::RoundRobin => vec![self.round_robin(n)],
        }
    }

    fn round_robin(&self, n: usize) -> u64 {
        let pos = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
        self.sub_conns[pos]
    }
}

/// Snapshot of the logical-connection state, swapped atomically on every write.
#[derive(Debug, Default, Clone)]
struct Snapshot {
    /// Logical id → logical connection.
    logical: HashMap<u64, LogicalConn>,
    /// Grouping key → logical id.
    by_key: HashMap<String, u64>,
    /// Physical id → logical id. Only grouped connections appear here.
    phys_to_logical: HashMap<u64, u64>,
}

/// What happened when a physical connection was detached from its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetachOutcome {
    /// The logical connection the physical connection belonged to. Equals the
    /// physical id when it was never grouped.
    pub logical_id: u64,
    /// True when the detached connection was the last sub-connection, meaning the
    /// logical connection is gone and its routing state must be torn down.
    pub was_last: bool,
}

/// Registry mapping physical connections to logical connections.
///
/// Concurrency mirrors [`ConnectionTable`](crate::tables::connection_table::ConnectionTable):
/// reads are lock-free through `ArcSwap`, writes are serialised by a mutex and
/// publish a new snapshot. Reads dominate — every forwarded message resolves at
/// least one id — while writes only happen on connect and disconnect.
#[derive(Debug)]
pub struct LogicalConnectionTable {
    snapshot: ArcSwap<Snapshot>,
    write_lock: Mutex<u64>,
}

impl Default for LogicalConnectionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl LogicalConnectionTable {
    pub fn new() -> Self {
        LogicalConnectionTable {
            snapshot: ArcSwap::from_pointee(Snapshot::default()),
            write_lock: Mutex::new(LOGICAL_ID_BASE),
        }
    }

    /// Group `physical` under the logical connection identified by `key`, creating
    /// it if this is the first sub-connection. Returns the logical id.
    ///
    /// `policy` is applied when the logical connection is created; later attaches
    /// with a different policy log a warning and keep the original, so that the
    /// behaviour of an established peer link does not change under it.
    pub fn attach(&self, physical: u64, key: &str, policy: SubConnPolicy) -> u64 {
        let mut next_id = self.write_lock.lock();
        let mut snap = (**self.snapshot.load()).clone();

        let logical_id = match snap.by_key.entry(key.to_string()) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let id = *next_id;
                *next_id += 1;
                e.insert(id);
                snap.logical.insert(
                    id,
                    LogicalConn {
                        key: key.to_string(),
                        sub_conns: Vec::new(),
                        policy,
                        cursor: AtomicUsize::new(0),
                    },
                );
                id
            }
        };

        // `logical` and `by_key` are written together, so the entry always exists.
        let conn = snap
            .logical
            .get_mut(&logical_id)
            .expect("logical entry missing for known key");

        if conn.policy != policy {
            tracing::warn!(
                %physical, %key, existing = %conn.policy, requested = %policy,
                "sub-connection policy differs from the established logical connection; keeping the existing policy",
            );
        }

        if !conn.sub_conns.contains(&physical) {
            conn.sub_conns.push(physical);
        }
        let count = conn.sub_conns.len();

        snap.phys_to_logical.insert(physical, logical_id);
        self.snapshot.store(Arc::new(snap));

        debug!(
            %physical, %logical_id, %key, policy = %policy, sub_conns = count,
            "attached sub-connection to logical connection",
        );

        logical_id
    }

    /// Remove `physical` from its logical connection.
    ///
    /// For an ungrouped connection this reports `logical_id == physical` and
    /// `was_last == true`, so callers can treat grouped and ungrouped connections
    /// through the same teardown path.
    pub fn detach(&self, physical: u64) -> DetachOutcome {
        let _guard = self.write_lock.lock();
        let mut snap = (**self.snapshot.load()).clone();

        let Some(logical_id) = snap.phys_to_logical.remove(&physical) else {
            return DetachOutcome {
                logical_id: physical,
                was_last: true,
            };
        };

        let mut was_last = false;
        if let Some(conn) = snap.logical.get_mut(&logical_id) {
            conn.sub_conns.retain(|&c| c != physical);
            if conn.sub_conns.is_empty() {
                let key = conn.key.clone();
                snap.logical.remove(&logical_id);
                snap.by_key.remove(&key);
                was_last = true;
            }
        }

        self.snapshot.store(Arc::new(snap));

        debug!(
            %physical, %logical_id, %was_last,
            "detached sub-connection from logical connection",
        );

        DetachOutcome {
            logical_id,
            was_last,
        }
    }

    /// The routing id for `physical`: its logical id when grouped, otherwise
    /// `physical` itself.
    ///
    /// This is the canonicalisation every routing-table access goes through. It is
    /// idempotent — passing an already-logical id returns it unchanged — so it is
    /// safe to apply at more than one point on the same path.
    pub fn resolve(&self, physical: u64) -> u64 {
        if is_logical(physical) {
            return physical;
        }
        self.snapshot
            .load()
            .phys_to_logical
            .get(&physical)
            .copied()
            .unwrap_or(physical)
    }

    /// The physical connections to send a message addressed to `id` on.
    ///
    /// A physical `id` is returned as-is, which keeps the send path working for
    /// local applications and for control-plane messages that name a concrete
    /// connection. A logical `id` is expanded through its policy. An empty result
    /// means the logical connection has no live sub-connections left.
    pub fn select(&self, id: u64, affinity_key: Option<u64>) -> Vec<u64> {
        if !is_logical(id) {
            return vec![id];
        }
        match self.snapshot.load().logical.get(&id) {
            Some(conn) => conn.select(affinity_key),
            None => Vec::new(),
        }
    }

    /// All sub-connections of `id`, ignoring policy. A physical id yields itself.
    pub fn sub_conns(&self, id: u64) -> Vec<u64> {
        if !is_logical(id) {
            return vec![id];
        }
        self.snapshot
            .load()
            .logical
            .get(&id)
            .map(|c| c.sub_conns.clone())
            .unwrap_or_default()
    }

    /// Number of sub-connections grouped under `id`.
    pub fn sub_conn_count(&self, id: u64) -> usize {
        if !is_logical(id) {
            return 1;
        }
        self.snapshot
            .load()
            .logical
            .get(&id)
            .map(|c| c.sub_conns.len())
            .unwrap_or(0)
    }

    /// The policy in force for `id`, or `None` when `id` is not a live logical
    /// connection.
    pub fn policy(&self, id: u64) -> Option<SubConnPolicy> {
        if !is_logical(id) {
            return None;
        }
        self.snapshot.load().logical.get(&id).map(|c| c.policy)
    }

    /// Number of live logical connections.
    pub fn len(&self) -> usize {
        self.snapshot.load().logical.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ungrouped_physical_id_resolves_to_itself() {
        let t = LogicalConnectionTable::new();
        assert_eq!(t.resolve(7), 7);
        assert_eq!(t.select(7, None), vec![7]);
        assert_eq!(t.sub_conn_count(7), 1);
        assert!(t.policy(7).is_none());
    }

    #[test]
    fn logical_ids_never_collide_with_physical_ids() {
        let t = LogicalConnectionTable::new();
        let id = t.attach(0, "peer-a", SubConnPolicy::RoundRobin);
        assert!(is_logical(id));
        assert!(id >= LOGICAL_ID_BASE);
        assert!(!is_logical(0));
        assert!(!is_logical(u32::MAX as u64));
    }

    #[test]
    fn same_key_groups_under_one_logical_id() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        let b = t.attach(2, "peer-a", SubConnPolicy::RoundRobin);
        assert_eq!(a, b);
        assert_eq!(t.len(), 1);
        assert_eq!(t.sub_conn_count(a), 2);
        assert_eq!(t.resolve(1), a);
        assert_eq!(t.resolve(2), a);
    }

    #[test]
    fn different_keys_get_different_logical_ids() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        let b = t.attach(2, "peer-b", SubConnPolicy::RoundRobin);
        assert_ne!(a, b);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn resolve_is_idempotent() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        assert_eq!(t.resolve(t.resolve(1)), a);
    }

    #[test]
    fn attaching_the_same_physical_twice_does_not_duplicate() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        assert_eq!(t.attach(1, "peer-a", SubConnPolicy::RoundRobin), a);
        assert_eq!(t.sub_conn_count(a), 1);
    }

    #[test]
    fn round_robin_cycles_through_sub_conns() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        t.attach(2, "peer-a", SubConnPolicy::RoundRobin);
        t.attach(3, "peer-a", SubConnPolicy::RoundRobin);

        let picks: Vec<u64> = (0..6).map(|_| t.select(a, None)[0]).collect();
        assert_eq!(picks, vec![1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn redundant_returns_every_sub_conn() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::Redundant);
        t.attach(2, "peer-a", SubConnPolicy::Redundant);
        t.attach(3, "peer-a", SubConnPolicy::Redundant);

        let mut out = t.select(a, None);
        out.sort();
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn failover_pins_to_the_first_sub_conn() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::Failover);
        t.attach(2, "peer-a", SubConnPolicy::Failover);

        assert_eq!(t.select(a, None), vec![1]);
        assert_eq!(t.select(a, None), vec![1]);

        // Primary goes away: traffic moves to the survivor, logical conn stays alive.
        let outcome = t.detach(1);
        assert_eq!(outcome.logical_id, a);
        assert!(!outcome.was_last);
        assert_eq!(t.select(a, None), vec![2]);
    }

    #[test]
    fn affinity_pins_a_flow_and_spreads_across_flows() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::Affinity);
        t.attach(2, "peer-a", SubConnPolicy::Affinity);
        t.attach(3, "peer-a", SubConnPolicy::Affinity);

        // Same key always lands on the same sub-connection.
        let first = t.select(a, Some(42));
        for _ in 0..5 {
            assert_eq!(t.select(a, Some(42)), first);
        }

        // Consecutive keys spread across the sub-connections.
        let spread: Vec<u64> = (0..3).map(|k| t.select(a, Some(k))[0]).collect();
        assert_eq!(spread, vec![1, 2, 3]);
    }

    #[test]
    fn affinity_without_a_key_falls_back_to_round_robin() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::Affinity);
        t.attach(2, "peer-a", SubConnPolicy::Affinity);

        let picks: Vec<u64> = (0..4).map(|_| t.select(a, None)[0]).collect();
        assert_eq!(picks, vec![1, 2, 1, 2]);
    }

    #[test]
    fn single_sub_conn_ignores_policy() {
        for policy in [
            SubConnPolicy::RoundRobin,
            SubConnPolicy::Redundant,
            SubConnPolicy::Affinity,
            SubConnPolicy::Failover,
        ] {
            let t = LogicalConnectionTable::new();
            let a = t.attach(1, "peer-a", policy);
            assert_eq!(t.select(a, None), vec![1], "policy {policy}");
            assert_eq!(t.select(a, Some(9)), vec![1], "policy {policy}");
        }
    }

    #[test]
    fn detach_reports_last_only_when_group_empties() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        t.attach(2, "peer-a", SubConnPolicy::RoundRobin);

        let first = t.detach(1);
        assert_eq!(first.logical_id, a);
        assert!(!first.was_last);
        assert_eq!(t.len(), 1);
        // The surviving sub-connection still resolves to the same logical id, so
        // routing state registered under it stays valid.
        assert_eq!(t.resolve(2), a);

        let second = t.detach(2);
        assert_eq!(second.logical_id, a);
        assert!(second.was_last);
        assert_eq!(t.len(), 0);
        assert_eq!(t.resolve(2), 2);
        assert!(t.select(a, None).is_empty());
    }

    #[test]
    fn detach_of_ungrouped_conn_reports_itself_as_last() {
        let t = LogicalConnectionTable::new();
        let outcome = t.detach(5);
        assert_eq!(outcome.logical_id, 5);
        assert!(outcome.was_last);
    }

    #[test]
    fn key_is_reusable_after_the_group_empties() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        assert!(t.detach(1).was_last);

        // A fresh logical id, so any stale routing state keyed on the old id can
        // never be mistaken for the new connection.
        let b = t.attach(1, "peer-a", SubConnPolicy::RoundRobin);
        assert_ne!(a, b);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn policy_of_established_group_wins() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a", SubConnPolicy::Redundant);
        t.attach(2, "peer-a", SubConnPolicy::RoundRobin);
        assert_eq!(t.policy(a), Some(SubConnPolicy::Redundant));
        assert_eq!(t.sub_conn_count(a), 2);
    }

    #[test]
    fn select_on_unknown_logical_id_is_empty() {
        let t = LogicalConnectionTable::new();
        assert!(t.select(LOGICAL_ID_BASE + 99, None).is_empty());
        assert_eq!(t.sub_conn_count(LOGICAL_ID_BASE + 99), 0);
    }
}
