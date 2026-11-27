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
//! Sends use one live sub-connection (the first in attach order that is not
//! down). A sub-connection is down from the moment its stream ends until it is
//! re-attached after a successful reconnect, so siblings carry the traffic in
//! between. Configurable selection policies are intentionally out of scope here.
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

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tracing::debug;

/// First logical connection id. Physical ids are `Pool` indices and never reach
/// this value, so the two id spaces cannot collide.
pub const LOGICAL_ID_BASE: u64 = 1 << 63;

/// Whether `id` denotes a logical connection rather than a physical one.
pub const fn is_logical(id: u64) -> bool {
    id >= LOGICAL_ID_BASE
}

/// A logical connection: the set of physical connections reaching one remote node.
#[derive(Debug, Clone)]
struct LogicalConn {
    /// Grouping key — the remote's advertised identity.
    key: String,
    /// Physical connection ids, in attach order. Sends use the first live entry.
    sub_conns: Vec<u64>,
    /// Sub-connections whose stream ended and that are waiting to reconnect.
    down: HashSet<u64>,
}

impl LogicalConn {
    fn select(&self) -> Option<u64> {
        self.sub_conns
            .iter()
            .copied()
            .find(|c| !self.down.contains(c))
    }

    fn remove(&mut self, physical: u64) {
        self.sub_conns.retain(|&c| c != physical);
        self.down.remove(&physical);
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

/// What happened when a physical connection was attached to a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachOutcome {
    /// The logical connection the physical connection now belongs to.
    pub logical_id: u64,
    /// The group the physical connection was moved out of, when that left it
    /// empty. Its routing state is stale and must be torn down.
    pub emptied: Option<u64>,
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
    /// it if this is the first sub-connection, and mark it live.
    ///
    /// A reconnect reuses its physical id, and may reach a restarted remote
    /// with a new key. `physical` is then moved out of its previous group, and
    /// that group is removed if it is left empty.
    pub fn attach(&self, physical: u64, key: &str) -> AttachOutcome {
        let mut next_id = self.write_lock.lock();
        let mut snap = (**self.snapshot.load()).clone();

        let emptied = Self::leave_previous_group(&mut snap, physical, key);

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
                        down: HashSet::new(),
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

        if !conn.sub_conns.contains(&physical) {
            conn.sub_conns.push(physical);
        }
        conn.down.remove(&physical);
        let count = conn.sub_conns.len();

        snap.phys_to_logical.insert(physical, logical_id);
        self.snapshot.store(Arc::new(snap));

        debug!(
            %physical, %logical_id, %key, sub_conns = count, ?emptied,
            "attached sub-connection to logical connection",
        );

        AttachOutcome {
            logical_id,
            emptied,
        }
    }

    /// Remove `physical` from the group it is in, if that group's key is not
    /// `key`. Returns the group's id when this left it empty.
    fn leave_previous_group(snap: &mut Snapshot, physical: u64, key: &str) -> Option<u64> {
        let previous = *snap.phys_to_logical.get(&physical)?;
        let conn = snap.logical.get_mut(&previous)?;
        if conn.key == key {
            return None;
        }
        conn.remove(physical);
        snap.phys_to_logical.remove(&physical);
        if !conn.sub_conns.is_empty() {
            return None;
        }
        let old_key = conn.key.clone();
        snap.logical.remove(&previous);
        snap.by_key.remove(&old_key);
        Some(previous)
    }

    /// Mark `physical` as down so sends skip it until it is re-attached.
    ///
    /// Returns false when `physical` is not grouped, which leaves nothing to do:
    /// an ungrouped connection is its own only route.
    pub fn mark_down(&self, physical: u64) -> bool {
        let _guard = self.write_lock.lock();
        let mut snap = (**self.snapshot.load()).clone();

        let Some(&logical_id) = snap.phys_to_logical.get(&physical) else {
            return false;
        };
        let Some(conn) = snap.logical.get_mut(&logical_id) else {
            return false;
        };
        conn.down.insert(physical);
        self.snapshot.store(Arc::new(snap));

        debug!(%physical, %logical_id, "sub-connection is down");
        true
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
            conn.remove(physical);
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

    /// The physical connection a message addressed to `id` should be written to.
    ///
    /// A physical `id` is returned as-is. A logical `id` expands to the first
    /// sub-connection that is not down. `None` means the logical connection has
    /// no live sockets.
    pub fn select(&self, id: u64) -> Option<u64> {
        if !is_logical(id) {
            return Some(id);
        }
        self.snapshot
            .load()
            .logical
            .get(&id)
            .and_then(|conn| conn.select())
    }

    /// All sub-connections of `id`, ignoring send selection. A physical id yields
    /// itself. Used by route listing, which must look up sockets in the connection
    /// table rather than the logical id.
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

    /// Another sub-connection in the same group as `physical`, preferring one
    /// that is not down. `None` when `physical` is ungrouped or alone.
    pub fn sibling(&self, physical: u64) -> Option<u64> {
        let snap = self.snapshot.load();
        let conn = snap.logical.get(snap.phys_to_logical.get(&physical)?)?;
        let mut others = conn.sub_conns.iter().copied().filter(|&c| c != physical);
        let first = others.clone().next();
        others.find(|c| !conn.down.contains(c)).or(first)
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
        assert_eq!(t.select(7), Some(7));
        assert_eq!(t.sub_conn_count(7), 1);
        assert_eq!(t.sub_conns(7), vec![7]);
    }

    #[test]
    fn logical_ids_never_collide_with_physical_ids() {
        let t = LogicalConnectionTable::new();
        let id = t.attach(0, "peer-a").logical_id;
        assert!(is_logical(id));
        assert!(id >= LOGICAL_ID_BASE);
        assert!(!is_logical(0));
        assert!(!is_logical(u32::MAX as u64));
    }

    #[test]
    fn same_key_groups_under_one_logical_id() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        let b = t.attach(2, "peer-a").logical_id;
        assert_eq!(a, b);
        assert_eq!(t.len(), 1);
        assert_eq!(t.sub_conn_count(a), 2);
        assert_eq!(t.resolve(1), a);
        assert_eq!(t.resolve(2), a);
    }

    #[test]
    fn different_keys_get_different_logical_ids() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        let b = t.attach(2, "peer-b").logical_id;
        assert_ne!(a, b);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn resolve_is_idempotent() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        assert_eq!(t.resolve(t.resolve(1)), a);
    }

    #[test]
    fn attaching_the_same_physical_twice_does_not_duplicate() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        assert_eq!(t.attach(1, "peer-a").logical_id, a);
        assert_eq!(t.sub_conn_count(a), 1);
    }

    #[test]
    fn send_uses_the_first_live_sub_connection() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        t.attach(2, "peer-a");
        t.attach(3, "peer-a");

        assert_eq!(t.select(a), Some(1));
        assert_eq!(t.sub_conns(a), vec![1, 2, 3]);

        let outcome = t.detach(1);
        assert_eq!(outcome.logical_id, a);
        assert!(!outcome.was_last);
        assert_eq!(t.select(a), Some(2));
        assert_eq!(t.sub_conns(a), vec![2, 3]);
    }

    #[test]
    fn detach_reports_last_only_when_group_empties() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        t.attach(2, "peer-a");

        let first = t.detach(1);
        assert_eq!(first.logical_id, a);
        assert!(!first.was_last);
        assert_eq!(t.len(), 1);
        assert_eq!(t.resolve(2), a);

        let second = t.detach(2);
        assert_eq!(second.logical_id, a);
        assert!(second.was_last);
        assert_eq!(t.len(), 0);
        assert_eq!(t.resolve(2), 2);
        assert!(t.select(a).is_none());
        assert!(t.sub_conns(a).is_empty());
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
        let a = t.attach(1, "peer-a").logical_id;
        assert!(t.detach(1).was_last);

        let b = t.attach(1, "peer-a").logical_id;
        assert_ne!(a, b);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn select_on_unknown_logical_id_is_empty() {
        let t = LogicalConnectionTable::new();
        assert!(t.select(LOGICAL_ID_BASE + 99).is_none());
        assert_eq!(t.sub_conn_count(LOGICAL_ID_BASE + 99), 0);
        assert!(t.sub_conns(LOGICAL_ID_BASE + 99).is_empty());
    }

    #[test]
    fn select_skips_a_sub_connection_that_is_down() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        t.attach(2, "peer-a");

        assert!(t.mark_down(1));
        assert_eq!(t.select(a), Some(2));
        assert_eq!(
            t.sub_conns(a),
            vec![1, 2],
            "a down sub-connection keeps its routes"
        );
        assert_eq!(t.resolve(1), a);
    }

    #[test]
    fn select_is_empty_when_every_sub_connection_is_down() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        t.attach(2, "peer-a");

        t.mark_down(1);
        t.mark_down(2);
        assert!(t.select(a).is_none());
    }

    #[test]
    fn mark_down_of_ungrouped_conn_is_a_no_op() {
        let t = LogicalConnectionTable::new();
        assert!(!t.mark_down(5));
        assert_eq!(t.select(5), Some(5));
    }

    #[test]
    fn reattach_with_the_same_key_marks_the_sub_connection_live_again() {
        let t = LogicalConnectionTable::new();
        let a = t.attach(1, "peer-a").logical_id;
        t.attach(2, "peer-a");
        t.mark_down(1);

        let outcome = t.attach(1, "peer-a");
        assert_eq!(outcome.logical_id, a);
        assert_eq!(outcome.emptied, None);
        assert_eq!(t.select(a), Some(1), "keeps its place in attach order");
        assert_eq!(t.sub_conns(a), vec![1, 2]);
    }

    #[test]
    fn reattach_with_a_new_key_leaves_and_removes_the_emptied_group() {
        let t = LogicalConnectionTable::new();
        let old = t.attach(1, "peer-a/instance-1").logical_id;
        t.mark_down(1);

        let outcome = t.attach(1, "peer-a/instance-2");
        assert_ne!(outcome.logical_id, old);
        assert_eq!(outcome.emptied, Some(old));
        assert!(t.sub_conns(old).is_empty());
        assert_eq!(t.resolve(1), outcome.logical_id);
        assert_eq!(t.select(outcome.logical_id), Some(1));
        assert_eq!(t.len(), 1);

        let detached = t.detach(1);
        assert_eq!(detached.logical_id, outcome.logical_id);
        assert!(detached.was_last);
        assert!(t.is_empty());
    }

    #[test]
    fn reattach_with_a_new_key_keeps_a_group_that_still_has_sub_connections() {
        let t = LogicalConnectionTable::new();
        let old = t.attach(1, "peer-a/instance-1").logical_id;
        t.attach(2, "peer-a/instance-1");

        let outcome = t.attach(1, "peer-a/instance-2");
        assert_eq!(outcome.emptied, None);
        assert_eq!(t.sub_conns(old), vec![2]);
        assert_eq!(t.sub_conns(outcome.logical_id), vec![1]);
        assert_eq!(t.resolve(2), old);
    }

    #[test]
    fn sibling_prefers_a_live_sub_connection() {
        let t = LogicalConnectionTable::new();
        t.attach(1, "peer-a");
        t.attach(2, "peer-a");
        t.attach(3, "peer-a");
        t.mark_down(2);

        assert_eq!(t.sibling(1), Some(3));
        t.mark_down(3);
        assert_eq!(t.sibling(1), Some(2), "falls back to a down sibling");
        assert_eq!(t.sibling(9), None);
    }

    #[test]
    fn sibling_of_a_lone_sub_connection_is_none() {
        let t = LogicalConnectionTable::new();
        t.attach(1, "peer-a");
        assert_eq!(t.sibling(1), None);
    }
}
