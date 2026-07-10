// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use tracing::{debug, warn};

use super::SubscriptionTable;
use super::{ConnType, MatchFilter};
use crate::api::{EncodedName, NameId, ProtoName};
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

    // Linear scan: acceptable for the expected connection counts (≤ 256).
    // Switch to a HashSet if this ever grows significantly larger.
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

    #[allow(dead_code)]
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

impl Clone for ConnList {
    fn clone(&self) -> Self {
        ConnList {
            conns: self.conns.clone(),
            cursor: AtomicUsize::new(self.cursor.load(Ordering::Relaxed)),
        }
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
// Invariant: `ids` and each element of `slots` always have the same length.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct PrefixEntry {
    /// Dense u128 array for name IDs.
    ids: Vec<u128>,
    /// Per-slot connection lists, indexed by `ConnType::index()`.
    slots: [Vec<ConnList>; ConnType::COUNT],
    /// Deduplicated union of all per-slot connections, indexed by `ConnType::index()`.
    aggregates: [Vec<u64>; ConnType::COUNT],
    /// Round-robin cursor for NULL_COMPONENT slot selection.
    slot_cursor: AtomicUsize,
    /// Human-readable prefix strings for for_each / Display / ProtoName.
    strings: [String; 3],
}

impl PrefixEntry {
    fn new(strings: [String; 3]) -> Self {
        PrefixEntry {
            ids: Vec::new(),
            slots: std::array::from_fn(|_| Vec::new()),
            aggregates: std::array::from_fn(|_| Vec::new()),
            slot_cursor: AtomicUsize::new(0),
            strings,
        }
    }

    /// Central accessor: returns the (slot list, aggregate list) pair for a category.
    fn lists_mut(&mut self, category: ConnType) -> (&mut Vec<ConnList>, &mut Vec<u64>) {
        let i = category.index();
        (&mut self.slots[i], &mut self.aggregates[i])
    }

    /// Append a new ID slot. Called only when `id` is not already present.
    fn push_id(&mut self, id: u128) {
        self.ids.push(id);
        for slot_vec in &mut self.slots {
            slot_vec.push(ConnList::new());
        }
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Rebuild aggregate connection lists from scratch (called after a slot is removed).
    fn rebuild_aggregates(&mut self) {
        for category in ConnType::ALL {
            let (slots, aggregate) = self.lists_mut(category);
            aggregate.clear();
            for list in slots.iter() {
                for &c in &list.conns {
                    if !aggregate.contains(&c) {
                        aggregate.push(c);
                    }
                }
            }
        }
    }

    /// True when all connection lists for slot `idx` are empty.
    fn is_slot_empty(&self, idx: usize) -> bool {
        self.slots.iter().all(|s| s[idx].is_empty())
    }

    /// Remove slot `idx` using swap_remove on every parallel Vec.
    fn remove_slot(&mut self, idx: usize) {
        self.ids.swap_remove(idx);
        for slot_vec in &mut self.slots {
            slot_vec.swap_remove(idx);
        }
        self.rebuild_aggregates();
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
    fn has_id(&self, id: u128) -> bool {
        self.ids.contains(&id)
    }

    /// Ensure a slot for `id` exists, then insert `conn` (deduped).
    /// Returns `true` if this was the first connection for `(id, category)` (0→1 transition).
    fn insert_conn_for_id(&mut self, id: u128, conn: u64, category: ConnType) -> bool {
        let idx = match self.ids.iter().position(|&i| i == id) {
            Some(i) => i,
            None => {
                self.push_id(id);
                self.ids.len() - 1
            }
        };

        let was_empty = self.slots[category.index()][idx].is_empty();

        let (slots, aggregate) = self.lists_mut(category);
        slots[idx].push(conn);
        if !aggregate.contains(&conn) {
            aggregate.push(conn);
        }

        was_empty
    }

    /// Remove `conn` from the slot for `id` (no-op if slot absent).
    /// Returns `true` if this was the last connection for `(id, category)` (1→0 transition).
    fn remove_conn_for_id(&mut self, id: u128, conn: u64, category: ConnType) -> bool {
        if let Some(idx) = self.ids.iter().position(|&i| i == id) {
            let (slots, aggregate) = self.lists_mut(category);
            slots[idx].remove(conn);
            if !slots.iter().any(|l| l.conns.contains(&conn))
                && let Some(pos) = aggregate.iter().position(|&c| c == conn)
            {
                aggregate.swap_remove(pos);
            }

            self.slots[category.index()][idx].is_empty()
        } else {
            false
        }
    }

    /// Return the per-slot ConnLists in filter priority order: local, peer, remote.
    fn filtered_slots(&self, idx: usize, filter: MatchFilter) -> impl Iterator<Item = &ConnList> {
        ConnType::ALL
            .into_iter()
            .filter(move |&ct| filter.includes(ct))
            .map(move |ct| &self.slots[ct.index()][idx])
    }

    /// Return the aggregate connection lists in filter priority order.
    fn filtered_aggregates(&self, filter: MatchFilter) -> impl Iterator<Item = &Vec<u64>> {
        ConnType::ALL
            .into_iter()
            .filter(move |&ct| filter.includes(ct))
            .map(move |ct| &self.aggregates[ct.index()])
    }

    /// Return one connection for `id`, respecting the filter, round-robin, excluding `skip`.
    /// Returns `None` if `id` has no slot or all connections equal `skip`.
    fn get_one(&self, id: u128, skip: u64, filter: MatchFilter) -> Option<u64> {
        let idx = self.ids.iter().position(|&i| i == id)?;
        self.filtered_slots(idx, filter)
            .find_map(|list| list.next(skip))
    }

    /// Return all connections for `id` respecting the filter, excluding `skip`.
    /// `None` means the slot for `id` does not exist;
    /// `Some(empty)` means the slot exists but all connections equal `skip`.
    fn get_all(&self, id: u128, skip: u64, filter: MatchFilter) -> Option<Vec<u64>> {
        let idx = self.ids.iter().position(|&i| i == id)?;
        Some(
            self.filtered_slots(idx, filter)
                .flat_map(|list| list.iter().filter(move |&c| c != skip))
                .collect(),
        )
    }

    /// NULL_COMPONENT `match_one`: pick one connection respecting the filter,
    /// round-robin across the pre-built aggregate lists, skipping `skip`.
    fn pick_one_any(&self, skip: u64, filter: MatchFilter) -> Option<u64> {
        for conns in self.filtered_aggregates(filter) {
            let n = conns.len();
            if n > 0 {
                let start = self.slot_cursor.fetch_add(1, Ordering::Relaxed) % n;
                for i in (start..n).chain(0..start) {
                    if conns[i] != skip {
                        return Some(conns[i]);
                    }
                }
            }
        }
        None
    }

    /// NULL_COMPONENT `match_all`: collect all distinct connections respecting the filter,
    /// skipping `skip`. Uses the pre-built aggregate lists (already deduped).
    fn get_all_any(&self, skip: u64, filter: MatchFilter) -> Vec<u64> {
        self.filtered_aggregates(filter)
            .flat_map(|conns| conns.iter().copied().filter(|&c| c != skip))
            .collect()
    }

    /// Iterate all slots, yielding `(id, local, remote, peer, edge)` — used by `for_each`.
    fn for_each_slot<F: FnMut(u128, &[u64], &[u64], &[u64], &[u64])>(&self, mut f: F) {
        for (i, &id) in self.ids.iter().enumerate() {
            f(
                id,
                self.slots[ConnType::Local.index()][i].as_slice(),
                self.slots[ConnType::Remote.index()][i].as_slice(),
                self.slots[ConnType::Peer.index()][i].as_slice(),
                self.slots[ConnType::Edge.index()][i].as_slice(),
            );
        }
    }

    /// Build a `ProtoName` from the stored strings and a component_3 value.
    fn to_proto_name(&self, id: u128) -> ProtoName {
        let base = ProtoName::from_strings([
            self.strings[0].as_str(),
            self.strings[1].as_str(),
            self.strings[2].as_str(),
        ]);
        if id != NameId::NULL_COMPONENT {
            base.with_id(id)
        } else {
            base
        }
    }
}

impl Clone for PrefixEntry {
    fn clone(&self) -> Self {
        PrefixEntry {
            ids: self.ids.clone(),
            slots: self.slots.clone(),
            aggregates: self.aggregates.clone(),
            slot_cursor: AtomicUsize::new(self.slot_cursor.load(Ordering::Relaxed)),
            strings: self.strings.clone(),
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
    category: ConnType,
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
    conn_subs: HashMap<u64, HashSet<u64>>,  // conn_id → sub_ids
}

impl SubscriptionState {
    /// Record a new subscription and update the reverse conn→sub_ids map.
    fn add(&mut self, sub_id: u64, record: SubRecord) {
        self.subscriptions.insert(sub_id, record);
        self.conn_subs
            .entry(record.conn_id)
            .or_default()
            .insert(sub_id);
    }

    /// Remove a subscription by `sub_id`, cleaning up `conn_subs` when empty.
    /// Returns the removed record, or `None` if the `sub_id` was not present.
    fn remove(&mut self, sub_id: u64) -> Option<SubRecord> {
        let record = self.subscriptions.remove(&sub_id)?;
        if let Some(set) = self.conn_subs.get_mut(&record.conn_id) {
            set.remove(&sub_id);
        }
        if self
            .conn_subs
            .get(&record.conn_id)
            .is_some_and(|s| s.is_empty())
        {
            self.conn_subs.remove(&record.conn_id);
        }
        Some(record)
    }

    /// True when `conn` still has at least one subscription for `encoded`.
    fn conn_still_subscribed(&self, conn: u64, encoded: EncodedName) -> bool {
        self.conn_subs.get(&conn).is_some_and(|subs| {
            subs.iter().any(|&s| {
                self.subscriptions
                    .get(&s)
                    .is_some_and(|r| r.encoded == encoded)
            })
        })
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// SubscriptionTableImpl
//
// Concurrency design:
//   - Hot read path (match_one / match_all): lock-free via ArcSwap::load().
//     Readers get an immutable snapshot with a single atomic pointer load.
//   - Write path (add/remove subscription, remove connection):
//       1. subscription_state.lock() — bookkeeping, always acquired first
//       2. load current routing snapshot
//       3. clone → modify → ArcSwap::store()  (no write lock held)
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct SubscriptionTableImpl {
    routing: ArcSwap<HashMap<[u64; 3], Arc<PrefixEntry>>>, // lock-free on the hot read path
    subscription_state: Mutex<SubscriptionState>,          // never held by match_one / match_all
}

impl Default for SubscriptionTableImpl {
    fn default() -> Self {
        SubscriptionTableImpl {
            routing: ArcSwap::from_pointee(HashMap::new()),
            subscription_state: Mutex::default(),
        }
    }
}

impl Display for SubscriptionTableImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let rs = self.routing.load();
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
                for ct in ConnType::ALL {
                    let label = match ct {
                        ConnType::Local => "Local",
                        ConnType::Remote => "Remote",
                        ConnType::Peer => "Peer",
                        ConnType::Edge => "Edge",
                    };
                    writeln!(f, "       {} Connections:", label)?;
                    let slot = &prefix_entry.slots[ct.index()][i];
                    if slot.is_empty() {
                        writeln!(f, "         None")?;
                    } else {
                        for c in slot.iter() {
                            writeln!(f, "         Connection: {}", c)?;
                        }
                    }
                }
                let peer_slot = &prefix_entry.slots[ConnType::Peer.index()][i];
                writeln!(f, "       Peer Connections:")?;
                if peer_slot.is_empty() {
                    writeln!(f, "         None")?;
                } else {
                    for c in peer_slot.iter() {
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
        F: FnMut(&ProtoName, u128, &[u64], &[u64], &[u64], &[u64]),
    {
        let rs = self.routing.load();
        for prefix_entry in rs.values() {
            let base_name = ProtoName::from_strings([
                prefix_entry.strings[0].as_str(),
                prefix_entry.strings[1].as_str(),
                prefix_entry.strings[2].as_str(),
            ]);
            prefix_entry.for_each_slot(|id, local, remote, peer, edge| {
                f(&base_name, id, local, remote, peer, edge);
            });
        }
    }

    fn add_subscription(
        &self,
        name: ProtoName,
        conn: u64,
        category: ConnType,
        subscription_id: u64,
    ) -> Result<bool, Self::Error> {
        let enc = name.name.as_ref().unwrap();
        let prefix = [enc.component_0, enc.component_1, enc.component_2];
        let id = enc.id();
        let encoded = *enc;

        // subscription_state locked first; routing updated via clone-modify-store.
        let mut ss = self.subscription_state.lock();
        let mut rs = (**self.routing.load()).clone();

        // 1. Ensure PrefixEntry exists; record strings on first insertion.
        let entry = rs.entry(prefix).or_insert_with(|| {
            let (s0, s1, s2) = name.str_components();
            Arc::new(PrefixEntry::new([
                s0.to_string(),
                s1.to_string(),
                s2.to_string(),
            ]))
        });

        // 2 & 3. Ensure an ID slot exists and insert conn (deduped).
        // Arc::make_mut does COW: clones only this PrefixEntry if other refs exist.
        let first = Arc::make_mut(entry).insert_conn_for_id(id, conn, category);
        self.routing.store(Arc::new(rs));

        // 4 & 5. Record subscription and update the reverse conn→sub_ids map.
        ss.add(
            subscription_id,
            SubRecord {
                encoded,
                conn_id: conn,
                category,
            },
        );

        debug!(%name, %conn, "subscription table: add subscription");
        Ok(first)
    }

    fn remove_subscription(
        &self,
        name: &ProtoName,
        conn: u64,
        category: ConnType,
        subscription_id: u64,
    ) -> Result<bool, Self::Error> {
        let enc = name.name.as_ref().unwrap();
        let prefix = [enc.component_0, enc.component_1, enc.component_2];
        let id = enc.id();
        let encoded = *enc;

        // subscription_state locked first; routing validated then updated via clone-modify-store.
        let mut ss = self.subscription_state.lock();

        // Error precedence: routing checks first (read snapshot, no clone yet).
        {
            let current = self.routing.load();
            if !current.contains_key(&prefix) {
                debug!("subscription not found {}", name);
                return Err(DataPathError::SubscriptionNotFound(name.clone()));
            }
            if !current[&prefix].has_id(id) {
                warn!(%id, "not found");
                let nid: NameId = id.into();
                let str_nid: String = nid.into();
                return Err(DataPathError::IdNotFound(str_nid));
            }
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

        // Remove sub_id from both maps (conn_subs is cleaned up when empty).
        ss.remove(subscription_id);

        // Check whether the connection is still subscribed to the same encoded name via another
        // subscription_id.
        let conn_still_subscribed = ss.conn_still_subscribed(conn, encoded);

        let last = if !conn_still_subscribed {
            let mut rs = (**self.routing.load()).clone();
            let (should_remove_prefix, last) = if let Some(entry) = rs.get_mut(&prefix) {
                let pe = Arc::make_mut(entry);
                let last = pe.remove_conn_for_id(id, conn, category);
                pe.retain_non_empty();
                (pe.is_empty(), last)
            } else {
                (false, false)
            };
            if should_remove_prefix {
                rs.remove(&prefix);
            }
            self.routing.store(Arc::new(rs));
            last
        } else {
            false
        };

        Ok(last)
    }

    fn remove_connection(
        &self,
        conn: u64,
        category: ConnType,
    ) -> Result<HashMap<ProtoName, HashSet<u64>>, Self::Error> {
        // subscription_state locked first; routing updated via clone-modify-store.
        let mut ss = self.subscription_state.lock();

        let sub_ids = ss
            .conn_subs
            .remove(&conn)
            .ok_or(DataPathError::ConnectionIdNotFound(conn))?;

        let mut result: HashMap<ProtoName, HashSet<u64>> = HashMap::with_capacity(sub_ids.len());

        // Pass 1: remove each subscription record and build the result map.
        // Use a read snapshot (no clone yet) for proto_name lookups.
        let mut encoded_names: Vec<EncodedName> = Vec::new();

        let current = self.routing.load();
        for sub_id in sub_ids {
            if let Some(record) = ss.subscriptions.remove(&sub_id) {
                let prefix = [
                    record.encoded.component_0,
                    record.encoded.component_1,
                    record.encoded.component_2,
                ];
                let proto_name = current
                    .get(&prefix)
                    .map(|pe| pe.to_proto_name(record.encoded.id()))
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

        // Pass 2: clone snapshot, remove conn from routing for every affected encoded name.
        let mut rs = (**current).clone();
        drop(current);
        for encoded in &encoded_names {
            debug!(%conn, ?encoded, "remove subscription");
            let prefix = [
                encoded.component_0,
                encoded.component_1,
                encoded.component_2,
            ];
            let id = encoded.id();

            let should_remove_prefix = if let Some(entry) = rs.get_mut(&prefix) {
                let pe = Arc::make_mut(entry);
                pe.remove_conn_for_id(id, conn, category);
                pe.retain_non_empty();
                pe.is_empty()
            } else {
                false
            };

            if should_remove_prefix {
                rs.remove(&prefix);
            }
        }
        self.routing.store(Arc::new(rs));

        Ok(result)
    }

    fn match_one(
        &self,
        encoded: &EncodedName,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<u64, Self::Error> {
        let prefix = [
            encoded.component_0,
            encoded.component_1,
            encoded.component_2,
        ];
        let id = encoded.id();
        let rs = self.routing.load();

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
                return Err(DataPathError::NoMatchEncoded(
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    encoded.string_id(),
                ));
            }
            Some(pe) => pe,
        };

        if id != NameId::NULL_COMPONENT {
            // Specific ID: linear scan over ids (8 per cache line).
            prefix_entry
                .get_one(id, incoming_conn, filter)
                .ok_or_else(|| {
                    debug!(component_3 = id, "match not found for name");
                    DataPathError::NoMatchEncoded(
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        encoded.string_id(),
                    )
                })
        } else {
            // NULL_COMPONENT: pick one connection across all registered IDs,
            // locals preferred.  Data is already hot from the single lookup above.
            prefix_entry
                .pick_one_any(incoming_conn, filter)
                .ok_or_else(|| {
                    debug!("no output connection available");
                    DataPathError::NoMatchEncoded(
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        encoded.string_id(),
                    )
                })
        }
    }

    fn match_all(
        &self,
        encoded: &EncodedName,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<Vec<u64>, Self::Error> {
        let prefix = [
            encoded.component_0,
            encoded.component_1,
            encoded.component_2,
        ];
        let id = encoded.id();
        let rs = self.routing.load();

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
                return Err(DataPathError::NoMatchEncoded(
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    encoded.string_id(),
                ));
            }
            Some(pe) => pe,
        };

        if id != NameId::NULL_COMPONENT {
            // Specific ID: linear scan over ids (8 per cache line).
            match prefix_entry.get_all(id, incoming_conn, filter) {
                None => {
                    debug!(component_3 = id, "match not found for name");
                    Err(DataPathError::NoMatchEncoded(
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        encoded.string_id(),
                    ))
                }
                Some(out) if out.is_empty() => {
                    debug!("no connection available (local/remote/peer)");
                    Err(DataPathError::NoMatchEncoded(
                        encoded.component_0,
                        encoded.component_1,
                        encoded.component_2,
                        encoded.string_id(),
                    ))
                }
                Some(out) => {
                    debug!(?out, "found connections");
                    Ok(out)
                }
            }
        } else {
            // NULL_COMPONENT: union of all connections respecting filter.
            // Data is already hot from the single lookup above.
            let out = prefix_entry.get_all_any(incoming_conn, filter);
            if out.is_empty() {
                debug!("no connection available");
                Err(DataPathError::NoMatchEncoded(
                    encoded.component_0,
                    encoded.component_1,
                    encoded.component_2,
                    encoded.string_id(),
                ))
            } else {
                debug!(?out, "found connections");
                Ok(out)
            }
        }
    }
}

impl SubscriptionTableImpl {
    /// Iterate all subscriptions, yielding `(ProtoName, sub_id, conn_id, category)` for each.
    ///
    /// This locks the subscription state and resolves names from the routing snapshot.
    /// Used by the sync module to collect subscriptions with their original IDs.
    pub fn for_each_subscription<F>(&self, mut f: F)
    where
        F: FnMut(ProtoName, u64, u64, ConnType),
    {
        let ss = self.subscription_state.lock();
        let rs = self.routing.load();
        for (&sub_id, record) in &ss.subscriptions {
            let prefix = [
                record.encoded.component_0,
                record.encoded.component_1,
                record.encoded.component_2,
            ];
            let proto_name = rs
                .get(&prefix)
                .map(|pe| pe.to_proto_name(record.encoded.id()))
                .unwrap_or(ProtoName {
                    name: Some(record.encoded),
                    str_name: None,
                });
            f(proto_name, sub_id, record.conn_id, record.category);
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

        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 2)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1_1.clone(), 3, ConnType::Remote, 3)
                .is_ok()
        );
        assert!(
            t.add_subscription(name2_2.clone(), 3, ConnType::Remote, 4)
                .is_ok()
        );

        // returns three matches on connection 1,2,3
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.contains(&1));
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        // return two matches on connection 2,3
        let out = t.match_all(&enc(&name1), 1, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        assert!(
            t.remove_subscription(&name1, 2, ConnType::Remote, 2)
                .is_ok()
        );

        // return two matches on connection 1,3
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&3));

        assert!(
            t.remove_subscription(&name1_1, 3, ConnType::Remote, 3)
                .is_ok()
        );

        // return one matches on connection 1
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        let err = t.match_all(&enc(&name1), 1, MatchFilter::ALL);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(..))));

        // add subscription again
        assert!(
            t.add_subscription(name1_1.clone(), 2, ConnType::Remote, 5)
                .is_ok()
        );

        // returns two matches on connection 1 and 2
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // run multiple times for randomness
        for _ in 0..20 {
            let out = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
            if out != 1 && out != 2 {
                // the output must be 1 or 2
                panic!("the output must be 1 or 2");
            }
        }

        // return connection 2
        let out = t.match_one(&enc(&name1_1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out, 2);

        // return connection 3
        let out = t.match_one(&enc(&name2_2), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out, 3);
        let removed_subs = t.remove_connection(2, ConnType::Remote).unwrap();
        assert_eq!(removed_subs.len(), 1);
        assert!(removed_subs.contains_key(&name1_1));

        // returns one match on connection 1
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        assert!(
            t.add_subscription(name2_2.clone(), 4, ConnType::Remote, 6)
                .is_ok()
        );

        // run multiple times for randomness
        for _ in 0..20 {
            let out = t.match_one(&enc(&name2_2), 100, MatchFilter::ALL).unwrap();
            if out != 3 && out != 4 {
                // the output must be 3 or 4
                panic!("the output must be 3 or 4");
            }
        }

        for _ in 0..20 {
            let out = t.match_one(&enc(&name2_2), 4, MatchFilter::ALL).unwrap();
            if out != 3 {
                // the output must be 3
                panic!("the output must be 3");
            }
        }

        assert!(
            t.remove_subscription(&name2_2, 4, ConnType::Remote, 6)
                .is_ok()
        );

        // test local vs remote
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Local, 7)
                .is_ok()
        );

        // returns both local (2) and remote (1) connections
        let out = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&1));

        // returns one match on connection 2
        let out = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out, 2);

        // returns both local (2) and remote (1) connections, excluding incoming connection (2)
        let out = t.match_all(&enc(&name1), 2, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // same here
        let out = t.match_one(&enc(&name1), 2, MatchFilter::ALL).unwrap();
        assert_eq!(out, 1);

        // test errors
        let err = t.remove_connection(4, ConnType::Remote);
        assert!(matches!(err, Err(DataPathError::ConnectionIdNotFound(_))));

        // Test that specific ID (name1_1) does NOT match NULL_COMPONENT subscriptions
        // At this point only name1 (NULL_COMPONENT) subscriptions exist on conn 1 and 2
        let err = t.match_one(&enc(&name1_1), 100, MatchFilter::ALL);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(..))));

        assert!(
            // this generates a warning
            t.add_subscription(name2_2.clone(), 3, ConnType::Remote, 8)
                .is_ok()
        );

        let err = t.remove_subscription(&name3, 2, ConnType::Remote, 9);
        assert!(matches!(err, Err(DataPathError::SubscriptionNotFound(_))));

        let err = t.remove_subscription(&name2, 2, ConnType::Remote, 10);
        assert!(matches!(err, Err(DataPathError::IdNotFound(_))));
    }

    #[test]
    fn test_iter() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "one"]);
        let name2 = ProtoName::from_strings(["agntcy", "default", "two"]);

        let t = SubscriptionTableImpl::default();

        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 2)
                .is_ok()
        );
        assert!(
            t.add_subscription(name2.clone(), 3, ConnType::Local, 3)
                .is_ok()
        );

        let mut h = HashMap::new();

        t.for_each(|k, id, local, remote, peer, edge| {
            println!(
                "key: {}, id: {}, local: {:?}, remote: {:?}, peer: {:?}, edge: {:?}",
                k, id, local, remote, peer, edge
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
        assert!(
            t.add_subscription(name.clone(), 1, ConnType::Local, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name.clone(), 2, ConnType::Local, 2)
                .is_ok()
        );

        // Add remote connections
        assert!(
            t.add_subscription(name.clone(), 3, ConnType::Remote, 3)
                .is_ok()
        );
        assert!(
            t.add_subscription(name.clone(), 4, ConnType::Remote, 4)
                .is_ok()
        );

        // Test match_all returns both local and remote connections
        let result = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
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
        let result = t.match_all(&enc(&name), 1, MatchFilter::ALL).unwrap();
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
        let result = t.match_all(&enc(&name), 3, MatchFilter::ALL).unwrap();
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
            let result = t.match_one(&enc(&name), 100, MatchFilter::ALL).unwrap();
            assert!(
                result == 1 || result == 2,
                "match_one should always prefer local connections"
            );
        }

        // Remove all local connections
        assert!(t.remove_subscription(&name, 1, ConnType::Local, 1).is_ok());
        assert!(t.remove_subscription(&name, 2, ConnType::Local, 2).is_ok());

        // Now match_all should only return remote connections
        let result = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 2, "Should return only 2 remote connections");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // And match_one should fall back to remote
        for _ in 0..20 {
            let result = t.match_one(&enc(&name), 100, MatchFilter::ALL).unwrap();
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
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 100)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 100)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 100)
                .is_ok()
        );

        let result = t
            .match_one(&enc(&name1), 100_u64, MatchFilter::ALL)
            .unwrap();
        assert_eq!(result, 1, "Should match to connection 1");

        // One remove is enough since it was deduped to a single entry
        assert!(
            t.remove_subscription(&name1, 1, ConnType::Remote, 100)
                .is_ok()
        );
        let err = t.match_one(&enc(&name1), 100_u64, MatchFilter::ALL);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(..))),
            "Subscription should be fully removed after removing its subscription_id"
        );

        // Test with multiple connections and subscription_ids
        let name2 = ProtoName::from_strings(["agntcy", "default", "multi"]);

        // Connection 1: 3 different subscription_ids = 3 refs
        assert!(
            t.add_subscription(name2.clone(), 1, ConnType::Remote, 201)
                .is_ok()
        );
        assert!(
            t.add_subscription(name2.clone(), 1, ConnType::Remote, 202)
                .is_ok()
        );
        assert!(
            t.add_subscription(name2.clone(), 1, ConnType::Remote, 203)
                .is_ok()
        );

        // Connection 2: 1 subscription_id
        assert!(
            t.add_subscription(name2.clone(), 2, ConnType::Remote, 204)
                .is_ok()
        );

        // Connection 3: 2 different subscription_ids
        assert!(
            t.add_subscription(name2.clone(), 3, ConnType::Remote, 205)
                .is_ok()
        );
        assert!(
            t.add_subscription(name2.clone(), 3, ConnType::Remote, 206)
                .is_ok()
        );

        // All three connections should be available
        let result = t
            .match_all(&enc(&name2), 100_u64, MatchFilter::ALL)
            .unwrap();
        assert_eq!(result.len(), 3, "Should have 3 connections");
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&3));

        // Remove connection 2's subscription
        assert!(
            t.remove_subscription(&name2, 2, ConnType::Remote, 204)
                .is_ok()
        );
        let result = t
            .match_all(&enc(&name2), 100_u64, MatchFilter::ALL)
            .unwrap();
        assert_eq!(
            result.len(),
            2,
            "Should have 2 connections after removing conn 2"
        );
        assert!(!result.contains(&2));

        // Remove one subscription_id from connection 1
        assert!(
            t.remove_subscription(&name2, 1, ConnType::Remote, 201)
                .is_ok()
        );
        // Connection 1 still has 2 more subscription_ids
        let result = t
            .match_all(&enc(&name2), 100_u64, MatchFilter::ALL)
            .unwrap();
        assert_eq!(result.len(), 2, "Should still have 2 connections");
        assert!(result.contains(&1));

        // Remove remaining subscription_ids from connection 1
        assert!(
            t.remove_subscription(&name2, 1, ConnType::Remote, 202)
                .is_ok()
        );
        assert!(
            t.remove_subscription(&name2, 1, ConnType::Remote, 203)
                .is_ok()
        );
        let result = t
            .match_all(&enc(&name2), 100_u64, MatchFilter::ALL)
            .unwrap();
        assert_eq!(result.len(), 1, "Should have 1 connection");
        assert!(result.contains(&3));

        // Remove connection 3's subscription_ids
        assert!(
            t.remove_subscription(&name2, 3, ConnType::Remote, 205)
                .is_ok()
        );
        assert!(
            t.remove_subscription(&name2, 3, ConnType::Remote, 206)
                .is_ok()
        );
        let err = t.match_one(&enc(&name2), 100_u64, MatchFilter::ALL);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(..))),
            "No connections should remain"
        );
    }

    #[test]
    #[traced_test]
    fn test_connection_death_with_refcounting() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "cleanup"]);
        let t = SubscriptionTableImpl::default();

        // Add subscription with different subscription_ids
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 301)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 302)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Remote, 303)
                .is_ok()
        );

        // Add another connection with single ref
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 304)
                .is_ok()
        );

        // Both connections should be available
        let result = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 2, "Should have 2 connections");

        // Connection 1 dies - should be force-removed regardless of ref count
        let removed = t.remove_connection(1, ConnType::Remote).unwrap();
        assert_eq!(removed.len(), 1, "Should have removed 1 subscription");
        assert!(removed.contains_key(&name1));
        // All three subscription IDs should have been preserved
        assert_eq!(removed[&name1], HashSet::from([301u64, 302, 303]));

        // Now only connection 2 should be available
        let result = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(
            result, 2,
            "Should only match to connection 2 after conn 1 dies"
        );

        let result = t.match_all(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 1, "Should only have 1 connection remaining");
        assert!(result.contains(&2));
    }

    #[test]
    #[traced_test]
    fn test_mixed_local_remote_refcounting() {
        let name1 = ProtoName::from_strings(["agntcy", "default", "mixed"]);
        let t = SubscriptionTableImpl::default();

        // Add local connection with different subscription_ids
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Local, 401)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 1, ConnType::Local, 402)
                .is_ok()
        );

        // Add remote connection with different subscription_ids
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 403)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 404)
                .is_ok()
        );
        assert!(
            t.add_subscription(name1.clone(), 2, ConnType::Remote, 405)
                .is_ok()
        );

        // Should prefer local connection
        for _ in 0..10 {
            let result = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
            assert_eq!(result, 1, "Should prefer local connection");
        }

        // Remove one local subscription_id - should still exist (has 402)
        assert!(
            t.remove_subscription(&name1, 1, ConnType::Local, 401)
                .is_ok()
        );
        let result = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result, 1, "Local connection should still exist");

        // Remove last local subscription_id - should be gone, fall back to remote
        assert!(
            t.remove_subscription(&name1, 1, ConnType::Local, 402)
                .is_ok()
        );
        for _ in 0..10 {
            let result = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
            assert_eq!(result, 2, "Should fall back to remote connection");
        }

        // Remove remote subscription_ids one by one
        assert!(
            t.remove_subscription(&name1, 2, ConnType::Remote, 403)
                .is_ok()
        );
        assert!(
            t.remove_subscription(&name1, 2, ConnType::Remote, 404)
                .is_ok()
        );
        // Still has one remaining
        let result = t.match_one(&enc(&name1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result, 2, "Remote should still exist with one sub");

        assert!(
            t.remove_subscription(&name1, 2, ConnType::Remote, 405)
                .is_ok()
        );
        let err = t.match_one(&enc(&name1), 100, MatchFilter::ALL);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(..))),
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
        assert!(
            t.add_subscription(name.clone(), 1, ConnType::Remote, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name.clone(), 2, ConnType::Remote, 2)
                .is_ok()
        );
        assert!(
            t.add_subscription(name_id1.clone(), 3, ConnType::Remote, 3)
                .is_ok()
        );
        assert!(
            t.add_subscription(name_id2.clone(), 4, ConnType::Remote, 4)
                .is_ok()
        );

        // Test 1: Message with NULL_COMPONENT should match ALL subscriptions
        // (both NULL_COMPONENT and specific IDs)
        let result = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
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
        let result = t.match_all(&enc(&name_id1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(
            result.len(),
            1,
            "Specific ID message should match only exact ID subscription"
        );
        assert!(result.contains(&3), "Should match only conn 3 (ID 1)");

        // Test 3: Message with specific ID 2 should match ONLY ID 2 subscription
        let result = t.match_all(&enc(&name_id2), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&4), "Should match only conn 4 (ID 2)");

        // Test 4: match_one should also respect these rules
        let result = t.match_one(&enc(&name_id1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result, 3, "Should return only the specific ID connection");

        // Test 5: Remove specific ID subscription, the match for message with that ID should fail
        assert!(
            t.remove_subscription(&name_id1, 3, ConnType::Remote, 3)
                .is_ok()
        );
        let err = t.match_one(&enc(&name_id1), 100, MatchFilter::ALL);
        assert!(
            matches!(err, Err(DataPathError::NoMatchEncoded(..))),
            "Specific ID message should NOT match NULL_COMPONENT subscriptions"
        );

        // Test 6: But NULL_COMPONENT message should still match remaining subscriptions
        let result = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 3, "Should match remaining subscriptions");
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&4));

        // Test 7: Remove all NULL_COMPONENT subscriptions
        assert!(t.remove_subscription(&name, 1, ConnType::Remote, 1).is_ok());
        assert!(t.remove_subscription(&name, 2, ConnType::Remote, 2).is_ok());

        // NULL_COMPONENT message should still match the specific ID subscription
        let result = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&4), "Should match ID 2 subscription");

        // Specific ID 2 message should still work
        let result = t.match_one(&enc(&name_id2), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result, 4);

        // But specific ID 1 should still fail (was removed earlier)
        let err = t.match_one(&enc(&name_id1), 100, MatchFilter::ALL);
        assert!(matches!(err, Err(DataPathError::NoMatchEncoded(..))));
    }

    #[test]
    #[traced_test]
    fn test_match_filter_exclude_peer() {
        let name = ProtoName::from_strings(["agntcy", "default", "svc"]);

        let t = SubscriptionTableImpl::default();

        // Subscribe on all three categories with distinct connections
        assert!(
            t.add_subscription(name.clone(), 10, ConnType::Local, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name.clone(), 20, ConnType::Remote, 2)
                .is_ok()
        );
        assert!(
            t.add_subscription(name.clone(), 30, ConnType::Peer, 3)
                .is_ok()
        );

        // MatchFilter::ALL returns all three connections
        let out = t.match_all(&enc(&name), 100, MatchFilter::ALL).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.contains(&10));
        assert!(out.contains(&20));
        assert!(out.contains(&30));

        // MatchFilter::EXCLUDE_PEER excludes the peer connection
        let out = t
            .match_all(&enc(&name), 100, MatchFilter::EXCLUDE_PEER)
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&10));
        assert!(out.contains(&20));
        assert!(!out.contains(&30));

        // match_one with EXCLUDE_PEER should never return the peer connection
        let result = t
            .match_one(&enc(&name), 100, MatchFilter::EXCLUDE_PEER)
            .unwrap();
        assert!(result == 10 || result == 20);

        // match_one with EXCLUDE_PEER skipping the local conn should return remote
        let result = t
            .match_one(&enc(&name), 10, MatchFilter::EXCLUDE_PEER)
            .unwrap();
        assert_eq!(result, 20);
    }

    #[test]
    #[traced_test]
    fn test_same_prefix_different_id_different_connections() {
        let name = ProtoName::from_strings(["agntcy", "default", "channel"]);
        let name_id1 = name.clone().with_id(1);
        let name_id2 = name.clone().with_id(2);

        let t = SubscriptionTableImpl::default();

        // Subscribe channel with ID 1 on connections 10 and 11
        assert!(
            t.add_subscription(name_id1.clone(), 10, ConnType::Local, 1)
                .is_ok()
        );
        assert!(
            t.add_subscription(name_id1.clone(), 11, ConnType::Remote, 2)
                .is_ok()
        );

        // Subscribe channel with ID 2 on connections 20, 21, and 22
        assert!(
            t.add_subscription(name_id2.clone(), 20, ConnType::Local, 3)
                .is_ok()
        );
        assert!(
            t.add_subscription(name_id2.clone(), 21, ConnType::Remote, 4)
                .is_ok()
        );
        assert!(
            t.add_subscription(name_id2.clone(), 22, ConnType::Remote, 5)
                .is_ok()
        );

        // Message for ID 1 should only match connections 10 and 11
        let result = t.match_all(&enc(&name_id1), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&10));
        assert!(result.contains(&11));
        assert!(!result.contains(&20));
        assert!(!result.contains(&21));
        assert!(!result.contains(&22));

        // Message for ID 2 should only match connections 20, 21, and 22
        let result = t.match_all(&enc(&name_id2), 100, MatchFilter::ALL).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains(&20));
        assert!(result.contains(&21));
        assert!(result.contains(&22));
        assert!(!result.contains(&10));
        assert!(!result.contains(&11));
    }
}
