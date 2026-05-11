// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::{Mutex, RwLock};
use tracing::{debug, warn};

use super::SubscriptionTable;
use crate::api::{EncodedName, ProtoName};
use crate::errors::DataPathError;

// ──────────────────────────────────────────────────────────────────────────────
// ConnList – a connection Vec with a built-in round-robin cursor.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ConnList {
    conns: Vec<u64>,
    cursor: AtomicUsize,
}

impl ConnList {
    fn new() -> Self {
        ConnList {
            conns: Vec::new(),
            cursor: AtomicUsize::new(0),
        }
    }

    /// Add `conn` if not already present (deduped).
    fn push(&mut self, conn: u64) {
        if !self.conns.contains(&conn) {
            self.conns.push(conn);
        }
    }

    /// Remove `conn` (no-op if absent).
    fn remove(&mut self, conn: u64) {
        if let Some(pos) = self.conns.iter().position(|&c| c == conn) {
            self.conns.swap_remove(pos);
        }
    }

    fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }

    fn len(&self) -> usize {
        self.conns.len()
    }

    fn as_slice(&self) -> &[u64] {
        &self.conns
    }

    fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.conns.iter().copied()
    }

    /// Round-robin next connection, skipping `skip`.
    fn next(&self, skip: u64) -> Option<u64> {
        let n = self.conns.len();
        if n == 0 {
            return None;
        }
        if n == 1 {
            return if self.conns[0] != skip {
                Some(self.conns[0])
            } else {
                None
            };
        }
        for _ in 0..n {
            let pos = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
            let c = self.conns[pos];
            if c != skip {
                return Some(c);
            }
        }
        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PrefixEntry – one allocation per unique [u64; 3] prefix.
//
// Struct-of-arrays layout: `ids` is a flat Vec<u64> so a scan for a matching
// component_3 value reads 8 IDs per cache line instead of striding across
// 72-byte AoS IdEntry objects.  Once the index is found the per-slot
// ConnLists are accessed by that index.
//
// Invariant: all three Vecs always have the same length.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct PrefixEntry {
    /// Dense u64 array — 8 IDs per cache line.
    ids: Vec<u64>,
    local: Vec<ConnList>,
    remote: Vec<ConnList>,
    /// Round-robin cursor for NULL_COMPONENT slot selection.
    slot_cursor: AtomicUsize,
    /// Human-readable prefix strings for for_each / Display / ProtoName.
    strings: [String; 3],
}

impl PrefixEntry {
    fn new(strings: [String; 3]) -> Self {
        PrefixEntry {
            ids: Vec::new(),
            local: Vec::new(),
            remote: Vec::new(),
            slot_cursor: AtomicUsize::new(0),
            strings,
        }
    }

    /// Append a new ID slot. Called only when `id` is not already present.
    fn push_id(&mut self, id: u64) {
        self.ids.push(id);
        self.local.push(ConnList::new());
        self.remote.push(ConnList::new());
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Add `conn` to the appropriate list for slot `idx` if not already present.
    fn insert_conn(&mut self, idx: usize, conn: u64, is_local: bool) {
        let list = if is_local {
            &mut self.local[idx]
        } else {
            &mut self.remote[idx]
        };
        list.push(conn);
    }

    /// Remove `conn` from the appropriate list for slot `idx` (no-op if absent).
    fn remove_conn(&mut self, idx: usize, conn: u64, is_local: bool) {
        let list = if is_local {
            &mut self.local[idx]
        } else {
            &mut self.remote[idx]
        };
        list.remove(conn);
    }

    /// True when both local and remote lists for slot `idx` are empty.
    fn is_slot_empty(&self, idx: usize) -> bool {
        self.local[idx].is_empty() && self.remote[idx].is_empty()
    }

    /// Remove slot `idx` using swap_remove on every parallel Vec.
    fn remove_slot(&mut self, idx: usize) {
        self.ids.swap_remove(idx);
        self.local.swap_remove(idx);
        self.remote.swap_remove(idx);
    }

    /// Remove all slots whose local and remote lists are both empty.
    /// Replaces `by_id.retain(|e| !e.is_empty())`.
    fn retain_non_empty(&mut self) {
        let mut i = 0;
        while i < self.ids.len() {
            if self.is_slot_empty(i) {
                self.remove_slot(i); // swap_remove: don't advance i
            } else {
                i += 1;
            }
        }
    }

    /// True when a slot for `id` exists.
    fn has_id(&self, id: u64) -> bool {
        self.ids.contains(&id)
    }

    /// Ensure a slot for `id` exists, then insert `conn` (deduped).
    fn insert_conn_for_id(&mut self, id: u64, conn: u64, is_local: bool) {
        if !self.ids.contains(&id) {
            self.push_id(id);
        }
        let idx = self.ids.iter().position(|&i| i == id).unwrap();
        self.insert_conn(idx, conn, is_local);
    }

    /// Remove `conn` from the slot for `id` (no-op if slot absent).
    fn remove_conn_for_id(&mut self, id: u64, conn: u64, is_local: bool) {
        if let Some(idx) = self.ids.iter().position(|&i| i == id) {
            self.remove_conn(idx, conn, is_local);
        }
    }

    /// Return one connection for `id`, preferring local, round-robin, excluding `skip`.
    /// Returns `None` if `id` has no slot or all connections equal `skip`.
    fn get_one(&self, id: u64, skip: u64) -> Option<u64> {
        let idx = self.ids.iter().position(|&i| i == id)?;
        self.local[idx]
            .next(skip)
            .or_else(|| self.remote[idx].next(skip))
    }

    /// Return all connections for `id` (local + remote) excluding `skip`.
    /// `None` means the slot for `id` does not exist;
    /// `Some(empty)` means the slot exists but all connections equal `skip`.
    fn get_all(&self, id: u64, skip: u64) -> Option<Vec<u64>> {
        let idx = self.ids.iter().position(|&i| i == id)?;
        let mut out = Vec::with_capacity(self.local[idx].len() + self.remote[idx].len());
        out.extend(
            self.local[idx]
                .iter()
                .chain(self.remote[idx].iter())
                .filter(|&c| c != skip),
        );
        Some(out)
    }

    /// NULL_COMPONENT `match_one`: pick one connection, all locals before any remote,
    /// starting from a random slot, skipping `skip`.
    fn pick_one_any(&self, skip: u64) -> Option<u64> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        // Skip the atomic increment when there is only one slot.
        let start = if n == 1 {
            0
        } else {
            self.slot_cursor.fetch_add(1, Ordering::Relaxed) % n
        };
        // All locals first (starting from random slot, wrapping).
        for i in (start..n).chain(0..start) {
            if let Some(c) = self.local[i].next(skip) {
                return Some(c);
            }
        }
        // Then all remotes (same wrapping start).
        for i in (start..n).chain(0..start) {
            if let Some(c) = self.remote[i].next(skip) {
                return Some(c);
            }
        }
        None
    }

    /// NULL_COMPONENT `match_all`: collect all distinct connections, locals before
    /// remotes, skipping `skip`.
    fn get_all_any(&self, skip: u64) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::with_capacity(self.len());
        for local in &self.local {
            for c in local.iter() {
                if c != skip && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        for remote in &self.remote {
            for c in remote.iter() {
                if c != skip && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Iterate all slots, yielding `(id, local, remote)` — used by `for_each`.
    fn for_each_slot<F: FnMut(u64, &[u64], &[u64])>(&self, mut f: F) {
        for (i, &id) in self.ids.iter().enumerate() {
            f(id, self.local[i].as_slice(), self.remote[i].as_slice());
        }
    }

    /// Build a `ProtoName` from the stored strings and a component_3 value.
    fn to_proto_name(&self, id: u64) -> ProtoName {
        let base = ProtoName::from_strings([
            self.strings[0].as_str(),
            self.strings[1].as_str(),
            self.strings[2].as_str(),
        ]);
        if id != ProtoName::NULL_COMPONENT {
            base.with_id(id)
        } else {
            base
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// SubRecord – flat write-path record stored per subscription_id.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct SubRecord {
    encoded: EncodedName,
    conn_id: u64,
}

// ──────────────────────────────────────────────────────────────────────────────
// SubscriptionState — COLD write path.
//
// Protected by a Mutex (exclusive access always required). Never held by
// match_one / match_all, so subscription churn never blocks message routing.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct SubscriptionState {
    subscriptions: HashMap<u64, SubRecord>, // sub_id  → (encoded, conn_id)
    conn_subs: HashMap<u64, Vec<u64>>,      // conn_id → sub_ids
}

// ──────────────────────────────────────────────────────────────────────────────
// SubscriptionTableImpl
//
// Lock ordering (must always be respected to prevent deadlock):
//   1. subscription_state.lock() — acquired first by all writers
//   2. routing.write()           — acquired second by all writers
// Readers only ever acquire routing.read(); they never touch subscription_state.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct SubscriptionTableImpl {
    routing: RwLock<HashMap<[u64; 3], PrefixEntry>>, // only lock held on the hot read path
    subscription_state: Mutex<SubscriptionState>,    // never held by match_one / match_all
}

impl Display for SubscriptionTableImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let rs = self.routing.read();
        writeln!(f, "Subscription Table")?;
        for prefix_entry in rs.values() {
            writeln!(
                f,
                "Type: {}/{}/{}",
                prefix_entry.strings[0], prefix_entry.strings[1], prefix_entry.strings[2]
            )?;
            writeln!(f, "  Names:")?;
            for (i, &id) in prefix_entry.ids.iter().enumerate() {
                writeln!(f, "    Id: {}", id)?;
                if prefix_entry.local[i].is_empty() {
                    writeln!(f, "       Local Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Local Connections:")?;
                    for c in prefix_entry.local[i].iter() {
                        writeln!(f, "         Connection: {}", c)?;
                    }
                }
                if prefix_entry.remote[i].is_empty() {
                    writeln!(f, "       Remote Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Remote Connections:")?;
                    for c in prefix_entry.remote[i].iter() {
                        writeln!(f, "         Connection: {}", c)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl SubscriptionTable for SubscriptionTableImpl {
    type Error = DataPathError;

    fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&ProtoName, u64, &[u64], &[u64]),
    {
        let rs = self.routing.read();
        for prefix_entry in rs.values() {
            let base_name = ProtoName::from_strings([
                prefix_entry.strings[0].as_str(),
                prefix_entry.strings[1].as_str(),
                prefix_entry.strings[2].as_str(),
            ]);
            prefix_entry.for_each_slot(|id, local, remote| {
                f(&base_name, id, local, remote);
            });
        }
    }

    fn add_subscription(
        &self,
        name: ProtoName,
        conn: u64,
        is_local: bool,
        subscription_id: u64,
    ) -> Result<(), Self::Error> {
        let enc = name.name.as_ref().unwrap();
        let prefix = [enc.component_0, enc.component_1, enc.component_2];
        let id = enc.component_3;
        let encoded = *enc;

        // Lock order: write_state first, routing second.
        let mut ss = self.subscription_state.lock();
        let mut rs = self.routing.write();

        // 1. Ensure PrefixEntry exists; record strings on first insertion.
        let prefix_entry = rs.entry(prefix).or_insert_with(|| {
            let (s0, s1, s2) = name.str_components();
            PrefixEntry::new([s0.to_string(), s1.to_string(), s2.to_string()])
        });

        // 2 & 3. Ensure an ID slot exists and insert conn (deduped).
        prefix_entry.insert_conn_for_id(id, conn, is_local);

        // 4. Record the subscription (idempotent: same sub_id overwrites itself).
        ss.subscriptions.insert(
            subscription_id,
            SubRecord {
                encoded,
                conn_id: conn,
            },
        );

        // 5. Track sub_id → conn (deduped).
        let subs = ss.conn_subs.entry(conn).or_default();
        if !subs.contains(&subscription_id) {
            subs.push(subscription_id);
        }

        debug!(%name, %conn, "subscription table: add subscription");
        Ok(())
    }

    fn remove_subscription(
        &self,
        name: &ProtoName,
        conn: u64,
        is_local: bool,
        subscription_id: u64,
    ) -> Result<(), Self::Error> {
        let enc = name.name.as_ref().unwrap();
        let prefix = [enc.component_0, enc.component_1, enc.component_2];
        let id = enc.component_3;
        let encoded = *enc;

        // Lock order: write_state first, routing second.
        let mut ss = self.subscription_state.lock();
        let mut rs = self.routing.write();

        // Error precedence: routing checks first.
        if !rs.contains_key(&prefix) {
            debug!("subscription not found {}", name);
            return Err(DataPathError::SubscriptionNotFound(name.clone()));
        }
        if !rs[&prefix].has_id(id) {
            warn!(%id, "not found");
            return Err(DataPathError::IdNotFound(id));
        }

        // Validate the subscription record.
        let record = ss
            .subscriptions
            .get(&subscription_id)
            .copied()
            .ok_or(DataPathError::SubscriptionIdNotFound(subscription_id))?;

        if record.conn_id != conn {
            return Err(DataPathError::ConnectionIdNotFound(conn));
        }
        if record.encoded != encoded {
            return Err(DataPathError::SubscriptionIdNotFound(subscription_id));
        }

        // Remove sub_id from the subscriptions map and the reverse conn_subs map.
        ss.subscriptions.remove(&subscription_id);
        if let Some(subs) = ss.conn_subs.get_mut(&conn) {
            subs.retain(|&s| s != subscription_id);
        }

        // Check whether the connection is still subscribed to the same encoded name via another
        // subscription_id.
        let conn_still_subscribed = ss.conn_subs.get(&conn).is_some_and(|subs| {
            subs.iter().any(|&s| {
                ss.subscriptions
                    .get(&s)
                    .is_some_and(|r| r.encoded == encoded)
            })
        });

        if !conn_still_subscribed {
            // Remove conn from the routing entry; drop the mutable ref before
            // potentially calling routing.remove().
            let should_remove_prefix = if let Some(prefix_entry) = rs.get_mut(&prefix) {
                prefix_entry.remove_conn_for_id(id, conn, is_local);
                prefix_entry.retain_non_empty();
                prefix_entry.is_empty()
            } else {
                false
            };
            if should_remove_prefix {
                rs.remove(&prefix);
            }
        }

        // Clean up conn_subs if empty.
        if ss.conn_subs.get(&conn).is_some_and(|s| s.is_empty()) {
            ss.conn_subs.remove(&conn);
        }

        Ok(())
    }

    fn remove_connection(
        &self,
        conn: u64,
        is_local: bool,
    ) -> Result<HashMap<ProtoName, HashSet<u64>>, Self::Error> {
        // Lock order: write_state first, routing second.
        let mut ss = self.subscription_state.lock();
        let mut rs = self.routing.write();

        let sub_ids = ss
            .conn_subs
            .remove(&conn)
            .ok_or(DataPathError::ConnectionIdNotFound(conn))?;

        let mut result: HashMap<ProtoName, HashSet<u64>> = HashMap::with_capacity(sub_ids.len());

        // Pass 1: remove each subscription record and build the result map.
        // Track which encoded names need routing cleanup.
        let mut encoded_names: Vec<EncodedName> = Vec::new();

        for sub_id in sub_ids {
            if let Some(record) = ss.subscriptions.remove(&sub_id) {
                let prefix = [
                    record.encoded.component_0,
                    record.encoded.component_1,
                    record.encoded.component_2,
                ];
                let proto_name = rs
                    .get(&prefix)
                    .map(|pe| pe.to_proto_name(record.encoded.component_3))
                    .unwrap_or(ProtoName {
                        name: Some(record.encoded),
                        str_name: None,
                    });
                result.entry(proto_name).or_default().insert(sub_id);

                if !encoded_names.contains(&record.encoded) {
                    encoded_names.push(record.encoded);
                }
            }
        }

        // Pass 2: remove conn from routing for every affected encoded name.
        for encoded in &encoded_names {
            debug!(%conn, ?encoded, "remove subscription");
            let prefix = [
                encoded.component_0,
                encoded.component_1,
                encoded.component_2,
            ];
            let id = encoded.component_3;

            let should_remove_prefix = if let Some(prefix_entry) = rs.get_mut(&prefix) {
                prefix_entry.remove_conn_for_id(id, conn, is_local);
                prefix_entry.retain_non_empty();
                prefix_entry.is_empty()
            } else {
                false
            };

            if should_remove_prefix {
                rs.remove(&prefix);
            }
        }

        Ok(result)
    }

    fn match_one(&self, encoded: &EncodedName, incoming_conn: u64) -> Result<u64, Self::Error> {
        let prefix = [
            encoded.component_0,
            encoded.component_1,
            encoded.component_2,
        ];
        let id = encoded.component_3;
        let rs = self.routing.read();

        // ONE HashMap lookup — the same for any c3 value.
        let prefix_entry = match rs.get(&prefix) {
            None => {
                debug!(
                    component_0 = encoded.component_0,
                    component_1 = encoded.component_1,
                    component_2 = encoded.component_2,
                    component_3 = id,
                    "match not found for name"
                );
                return Err(DataPathError::NoMatchEncoded([
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    id,
                ]));
            }
            Some(pe) => pe,
        };

        if id != ProtoName::NULL_COMPONENT {
            // Specific ID: linear scan over ids (8 per cache line).
            prefix_entry.get_one(id, incoming_conn).ok_or_else(|| {
                debug!(component_3 = id, "match not found for name");
                DataPathError::NoMatchEncoded([
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    id,
                ])
            })
        } else {
            // NULL_COMPONENT: pick one connection across all registered IDs,
            // locals preferred.  Data is already hot from the single lookup above.
            prefix_entry.pick_one_any(incoming_conn).ok_or_else(|| {
                debug!("no output connection available");
                DataPathError::NoMatchEncoded([
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    id,
                ])
            })
        }
    }

    fn match_all(
        &self,
        encoded: &EncodedName,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, Self::Error> {
        let prefix = [
            encoded.component_0,
            encoded.component_1,
            encoded.component_2,
        ];
        let id = encoded.component_3;
        let rs = self.routing.read();

        // ONE HashMap lookup — the same for any c3 value.
        let prefix_entry = match rs.get(&prefix) {
            None => {
                debug!(
                    component_0 = encoded.component_0,
                    component_1 = encoded.component_1,
                    component_2 = encoded.component_2,
                    component_3 = id,
                    "match not found for name"
                );
                return Err(DataPathError::NoMatchEncoded([
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    id,
                ]));
            }
            Some(pe) => pe,
        };

        if id != ProtoName::NULL_COMPONENT {
            // Specific ID: linear scan over ids (8 per cache line).
            match prefix_entry.get_all(id, incoming_conn) {
                None => {
                    debug!(component_3 = id, "match not found for name");
                    Err(DataPathError::NoMatchEncoded([
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        id,
                    ]))
                }
                Some(out) if out.is_empty() => {
                    debug!("no connection available (local/remote)");
                    Err(DataPathError::NoMatchEncoded([
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        id,
                    ]))
                }
                Some(out) => {
                    debug!(?out, "found connections");
                    Ok(out)
                }
            }
        } else {
            // NULL_COMPONENT: union of all connections, locals before remotes.
            // Data is already hot from the single lookup above.
            let out = prefix_entry.get_all_any(incoming_conn);
            if out.is_empty() {
                debug!("no connection available");
                Err(DataPathError::NoMatchEncoded([
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    id,
                ]))
            } else {
                debug!(?out, "found connections");
                Ok(out)
            }
        }
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
    fn test_table() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "one"]);
        let name2 = ProtoName::from_strings(["agntcy", "default", "two"]);
        let name3 = ProtoName::from_strings(["agntcy", "default", "three"]);

        let name1_1 = name1.clone().with_id(1);
        let name2_2 = name2.clone().with_id(2);

        let t = SubscriptionTableImpl::default();

        assert!(t.add_subscription(name1.clone(), 1, false, 1).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false, 2).is_ok());
        assert!(t.add_subscription(name1_1.clone(), 3, false, 3).is_ok());
        assert!(t.add_subscription(name2_2.clone(), 3, false, 4).is_ok());

        // returns three matches on connection 1,2,3
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.contains(&1));
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        // return two matches on connection 2,3
        let out = t.match_all(&enc(&name1), 1).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        assert!(t.remove_subscription(&name1, 2, false, 2).is_ok());

        // return two matches on connection 1,3
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&3));

        assert!(t.remove_subscription(&name1_1, 3, false, 3).is_ok());

        // return one matches on connection 1
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        let err = t.match_all(&enc(&name1), 1);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(_))));

        // add subscription again
        assert!(t.add_subscription(name1_1.clone(), 2, false, 5).is_ok());

        // returns two matches on connection 1 and 2
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // run multiple times for randomnes
        for _ in 0..20 {
            let out = t.match_one(&enc(&name1), 100).unwrap();
            if out != 1 && out != 2 {
                // the output must be 1 or 2
                panic!("the output must be 1 or 2");
            }
        }

        // return connection 2
        let out = t.match_one(&enc(&name1_1), 100).unwrap();
        assert_eq!(out, 2);

        // return connection 3
        let out = t.match_one(&enc(&name2_2), 100).unwrap();
        assert_eq!(out, 3);
        let removed_subs = t.remove_connection(2, false).unwrap();
        assert_eq!(removed_subs.len(), 1);
        assert!(removed_subs.contains_key(&name1_1));

        // returns one match on connection 1
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        assert!(t.add_subscription(name2_2.clone(), 4, false, 6).is_ok());

        // run multiple times for randomness
        for _ in 0..20 {
            let out = t.match_one(&enc(&name2_2), 100).unwrap();
            if out != 3 && out != 4 {
                // the output must be 3 or 4
                panic!("the output must be 3 or 4");
            }
        }

        for _ in 0..20 {
            let out = t.match_one(&enc(&name2_2), 4).unwrap();
            if out != 3 {
                // the output must be 3
                panic!("the output must be 3");
            }
        }

        assert!(t.remove_subscription(&name2_2, 4, false, 6).is_ok());

        // test local vs remote
        assert!(t.add_subscription(name1.clone(), 2, true, 7).is_ok());

        // returns both local (2) and remote (1) connections
        let out = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&1));

        // returns one match on connection 2
        let out = t.match_one(&enc(&name1), 100).unwrap();
        assert_eq!(out, 2);

        // returns both local (2) and remote (1) connections, excluding incoming connection (2)
        let out = t.match_all(&enc(&name1), 2).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // same here
        let out = t.match_one(&enc(&name1), 2).unwrap();
        assert_eq!(out, 1);

        // test errors
        let err = t.remove_connection(4, false);
        assert!(matches!(err, Err(DataPathError::ConnectionIdNotFound(_))));

        // Test that specific ID (name1_1) does NOT match NULL_COMPONENT subscriptions
        // At this point only name1 (NULL_COMPONENT) subscriptions exist on conn 1 and 2
        let err = t.match_one(&enc(&name1_1), 100);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(_))));

        assert!(
            // this generates a warning
            t.add_subscription(name2_2.clone(), 3, false, 8).is_ok()
        );

        let err = t.remove_subscription(&name3, 2, false, 9);
        assert!(matches!(err, Err(DataPathError::SubscriptionNotFound(_))));

        let err = t.remove_subscription(&name2, 2, false, 10);
        assert!(matches!(err, Err(DataPathError::IdNotFound(_))));
    }

    #[test]
    fn test_iter() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "one"]);
        let name2 = ProtoName::from_strings(["agntcy", "default", "two"]);

        let t = SubscriptionTableImpl::default();

        assert!(t.add_subscription(name1.clone(), 1, false, 1).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false, 2).is_ok());
        assert!(t.add_subscription(name2.clone(), 3, true, 3).is_ok());

        let mut h = HashMap::new();

        t.for_each(|k, id, local, remote| {
            println!(
                "key: {}, id: {}, local: {:?}, remote: {:?}",
                k, id, local, remote
            );

            let mut local_sorted = local.to_vec();
            local_sorted.sort();
            let mut remote_sorted = remote.to_vec();
            remote_sorted.sort();
            h.insert(k.clone(), (id, local_sorted, remote_sorted));
        });

        assert_eq!(h.len(), 2);
        assert_eq!(h[&name1].1, vec![] as Vec<u64>);
        assert_eq!(h[&name1].2, vec![1, 2]);

        assert_eq!(h[&name2].1, vec![3]);
        assert_eq!(h[&name2].2, vec![] as Vec<u64>);
    }

    #[test]
    fn test_match_all_with_mixed_local_and_remote_connections() {
        let name = ProtoName::from_strings(["agntcy", "default", "service"]);
        let t = SubscriptionTableImpl::default();

        // Add local connections
        assert!(t.add_subscription(name.clone(), 1, true, 1).is_ok());
        assert!(t.add_subscription(name.clone(), 2, true, 2).is_ok());

        // Add remote connections
        assert!(t.add_subscription(name.clone(), 3, false, 3).is_ok());
        assert!(t.add_subscription(name.clone(), 4, false, 4).is_ok());

        // Test match_all returns both local and remote connections
        let result = t.match_all(&enc(&name), 100).unwrap();
        assert_eq!(
            result.len(),
            4,
            "Should return all 4 connections (2 local + 2 remote)"
        );
        assert!(result.contains(&1), "Should contain local connection 1");
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test excluding incoming connection works for local
        let result = t.match_all(&enc(&name), 1).unwrap();
        assert_eq!(
            result.len(),
            3,
            "Should return 3 connections (excluding conn 1)"
        );
        assert!(
            !result.contains(&1),
            "Should not contain incoming connection 1"
        );
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test excluding incoming connection works for remote
        let result = t.match_all(&enc(&name), 3).unwrap();
        assert_eq!(
            result.len(),
            3,
            "Should return 3 connections (excluding conn 3)"
        );
        assert!(result.contains(&1), "Should contain local connection 1");
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(
            !result.contains(&3),
            "Should not contain incoming connection 3"
        );
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test match_one prefers local over remote
        for _ in 0..20 {
            let result = t.match_one(&enc(&name), 100).unwrap();
            assert!(
                result == 1 || result == 2,
                "match_one should always prefer local connections"
            );
        }

        // Remove all local connections
        assert!(t.remove_subscription(&name, 1, true, 1).is_ok());
        assert!(t.remove_subscription(&name, 2, true, 2).is_ok());

        // Now match_all should only return remote connections
        let result = t.match_all(&enc(&name), 100).unwrap();
        assert_eq!(result.len(), 2, "Should return only 2 remote connections");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // And match_one should fall back to remote
        for _ in 0..20 {
            let result = t.match_one(&enc(&name), 100).unwrap();
            assert!(
                result == 3 || result == 4,
                "Should return remote connections"
            );
        }
    }

    #[test]
    #[traced_test]
    fn test_subscription_refcounting() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "service"]);
        let t = SubscriptionTableImpl::default();

        // Adding the same subscription_id multiple times is idempotent (dedup)
        assert!(t.add_subscription(name1.clone(), 1, false, 100).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false, 100).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false, 100).is_ok());

        let result = t.match_one(&enc(&name1), 100_u64).unwrap();
        assert_eq!(result, 1, "Should match to connection 1");

        // One remove is enough since it was deduped to a single entry
        assert!(t.remove_subscription(&name1, 1, false, 100).is_ok());
        let err = t.match_one(&enc(&name1), 100_u64);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(_))),
            "Subscription should be fully removed after removing its subscription_id"
        );

        // Test with multiple connections and subscription_ids
        let name2 = ProtoName::from_strings(["agntcy", "default", "multi"]);

        // Connection 1: 3 different subscription_ids = 3 refs
        assert!(t.add_subscription(name2.clone(), 1, false, 201).is_ok());
        assert!(t.add_subscription(name2.clone(), 1, false, 202).is_ok());
        assert!(t.add_subscription(name2.clone(), 1, false, 203).is_ok());

        // Connection 2: 1 subscription_id
        assert!(t.add_subscription(name2.clone(), 2, false, 204).is_ok());

        // Connection 3: 2 different subscription_ids
        assert!(t.add_subscription(name2.clone(), 3, false, 205).is_ok());
        assert!(t.add_subscription(name2.clone(), 3, false, 206).is_ok());

        // All three connections should be available
        let result = t.match_all(&enc(&name2), 100_u64).unwrap();
        assert_eq!(result.len(), 3, "Should have 3 connections");
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&3));

        // Remove connection 2's subscription
        assert!(t.remove_subscription(&name2, 2, false, 204).is_ok());
        let result = t.match_all(&enc(&name2), 100_u64).unwrap();
        assert_eq!(
            result.len(),
            2,
            "Should have 2 connections after removing conn 2"
        );
        assert!(!result.contains(&2));

        // Remove one subscription_id from connection 1
        assert!(t.remove_subscription(&name2, 1, false, 201).is_ok());
        // Connection 1 still has 2 more subscription_ids
        let result = t.match_all(&enc(&name2), 100_u64).unwrap();
        assert_eq!(result.len(), 2, "Should still have 2 connections");
        assert!(result.contains(&1));

        // Remove remaining subscription_ids from connection 1
        assert!(t.remove_subscription(&name2, 1, false, 202).is_ok());
        assert!(t.remove_subscription(&name2, 1, false, 203).is_ok());
        let result = t.match_all(&enc(&name2), 100_u64).unwrap();
        assert_eq!(result.len(), 1, "Should have 1 connection");
        assert!(result.contains(&3));

        // Remove connection 3's subscription_ids
        assert!(t.remove_subscription(&name2, 3, false, 205).is_ok());
        assert!(t.remove_subscription(&name2, 3, false, 206).is_ok());
        let err = t.match_one(&enc(&name2), 100_u64);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(_))),
            "No connections should remain"
        );
    }

    #[test]
    #[traced_test]
    fn test_connection_death_with_refcounting() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "cleanup"]);
        let t = SubscriptionTableImpl::default();

        // Add subscription with different subscription_ids
        assert!(t.add_subscription(name1.clone(), 1, false, 301).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false, 302).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false, 303).is_ok());

        // Add another connection with single ref
        assert!(t.add_subscription(name1.clone(), 2, false, 304).is_ok());

        // Both connections should be available
        let result = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(result.len(), 2, "Should have 2 connections");

        // Connection 1 dies - should be force-removed regardless of ref count
        let removed = t.remove_connection(1, false).unwrap();
        assert_eq!(removed.len(), 1, "Should have removed 1 subscription");
        assert!(removed.contains_key(&name1));
        // All three subscription IDs should have been preserved
        assert_eq!(removed[&name1], HashSet::from([301u64, 302, 303]));

        // Now only connection 2 should be available
        let result = t.match_one(&enc(&name1), 100).unwrap();
        assert_eq!(
            result, 2,
            "Should only match to connection 2 after conn 1 dies"
        );

        let result = t.match_all(&enc(&name1), 100).unwrap();
        assert_eq!(result.len(), 1, "Should only have 1 connection remaining");
        assert!(result.contains(&2));
    }

    #[test]
    #[traced_test]
    fn test_mixed_local_remote_refcounting() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "mixed"]);
        let t = SubscriptionTableImpl::default();

        // Add local connection with different subscription_ids
        assert!(t.add_subscription(name1.clone(), 1, true, 401).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, true, 402).is_ok());

        // Add remote connection with different subscription_ids
        assert!(t.add_subscription(name1.clone(), 2, false, 403).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false, 404).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false, 405).is_ok());

        // Should prefer local connection
        for _ in 0..10 {
            let result = t.match_one(&enc(&name1), 100).unwrap();
            assert_eq!(result, 1, "Should prefer local connection");
        }

        // Remove one local subscription_id - should still exist (has 402)
        assert!(t.remove_subscription(&name1, 1, true, 401).is_ok());
        let result = t.match_one(&enc(&name1), 100).unwrap();
        assert_eq!(result, 1, "Local connection should still exist");

        // Remove last local subscription_id - should be gone, fall back to remote
        assert!(t.remove_subscription(&name1, 1, true, 402).is_ok());
        for _ in 0..10 {
            let result = t.match_one(&enc(&name1), 100).unwrap();
            assert_eq!(result, 2, "Should fall back to remote connection");
        }

        // Remove remote subscription_ids one by one
        assert!(t.remove_subscription(&name1, 2, false, 403).is_ok());
        assert!(t.remove_subscription(&name1, 2, false, 404).is_ok());
        // Still has one remaining
        let result = t.match_one(&enc(&name1), 100).unwrap();
        assert_eq!(result, 2, "Remote should still exist with one sub");

        assert!(t.remove_subscription(&name1, 2, false, 405).is_ok());
        let err = t.match_one(&enc(&name1), 100);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(_))),
            "No connections should remain"
        );
    }

    #[test]
    #[traced_test]
    fn test_null_component_matching_behavior() {
        // This test validates the NULL_COMPONENT matching rules:
        // 1. Messages with NULL_COMPONENT match ANY subscription (NULL_COMPONENT or specific ID)
        // 2. Messages with specific ID match ONLY subscriptions with that exact ID

        let name = ProtoName::from_strings(["agntcy", "default", "service"]);
        let name_id1 = name.clone().with_id(1);
        let name_id2 = name.clone().with_id(2);

        let t = SubscriptionTableImpl::default();

        // Setup: Add subscriptions for NULL_COMPONENT and specific IDs
        // conn 1: NULL_COMPONENT (for discovery)
        // conn 2: NULL_COMPONENT (for discovery)
        // conn 3: specific ID 1
        // conn 4: specific ID 2
        assert!(t.add_subscription(name.clone(), 1, false, 1).is_ok());
        assert!(t.add_subscription(name.clone(), 2, false, 2).is_ok());
        assert!(t.add_subscription(name_id1.clone(), 3, false, 3).is_ok());
        assert!(t.add_subscription(name_id2.clone(), 4, false, 4).is_ok());

        // Test 1: Message with NULL_COMPONENT should match ALL subscriptions
        // (both NULL_COMPONENT and specific IDs)
        let result = t.match_all(&enc(&name), 100).unwrap();
        assert_eq!(
            result.len(),
            4,
            "NULL_COMPONENT message should match all subscriptions"
        );
        assert!(result.contains(&1), "Should match NULL_COMPONENT conn 1");
        assert!(result.contains(&2), "Should match NULL_COMPONENT conn 2");
        assert!(result.contains(&3), "Should match specific ID 1 conn 3");
        assert!(result.contains(&4), "Should match specific ID 2 conn 4");

        // Test 2: Message with specific ID 1 should match ONLY ID 1 subscription
        // (excluding NULL_COMPONENT subscriptions)
        let result = t.match_all(&enc(&name_id1), 100).unwrap();
        assert_eq!(
            result.len(),
            1,
            "Specific ID message should match only exact ID subscription"
        );
        assert!(result.contains(&3), "Should match only conn 3 (ID 1)");

        // Test 3: Message with specific ID 2 should match ONLY ID 2 subscription
        let result = t.match_all(&enc(&name_id2), 100).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&4), "Should match only conn 4 (ID 2)");

        // Test 4: match_one should also respect these rules
        let result = t.match_one(&enc(&name_id1), 100).unwrap();
        assert_eq!(result, 3, "Should return only the specific ID connection");

        // Test 5: Remove specific ID subscription, the match for message with that ID should fail
        assert!(t.remove_subscription(&name_id1, 3, false, 3).is_ok());
        let err = t.match_one(&enc(&name_id1), 100);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(_))),
            "Specific ID message should NOT match NULL_COMPONENT subscriptions"
        );

        // Test 6: But NULL_COMPONENT message should still match remaining subscriptions
        let result = t.match_all(&enc(&name), 100).unwrap();
        assert_eq!(result.len(), 3, "Should match remaining subscriptions");
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&4));

        // Test 7: Remove all NULL_COMPONENT subscriptions
        assert!(t.remove_subscription(&name, 1, false, 1).is_ok());
        assert!(t.remove_subscription(&name, 2, false, 2).is_ok());

        // NULL_COMPONENT message should still match the specific ID subscription
        let result = t.match_all(&enc(&name), 100).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&4), "Should match ID 2 subscription");

        // Specific ID 2 message should still work
        let result = t.match_one(&enc(&name_id2), 100).unwrap();
        assert_eq!(result, 4);

        // But specific ID 1 should still fail (was removed earlier)
        let err = t.match_one(&enc(&name_id1), 100);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(_))));
    }
}
