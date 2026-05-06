// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::trace;

/// Internal wrapper stored in `pool`.  Bundles the user value with the
/// element's current position in `active_indexes` so that `remove` can
/// find and patch that dense list in O(1) without scanning.
#[derive(Debug)]
struct PoolEntry<T> {
    value: T,
    /// Index of this slot's entry within `active_indexes`.
    active_pos: usize,
}

/// A collection that assigns each inserted element a stable `u64` ID.
///
/// IDs map directly to Vec indices, giving O(1) `get()` with no hash
/// computation.  Iteration is dense — only live elements are visited.
/// Freed slots are recycled via a free-list stack so `insert` is O(1)
/// amortised.
///
/// **Note:** IDs *are* reused after `remove` + `insert`.  Callers must
/// not retain an ID after calling `remove`.
#[derive(Debug)]
pub struct Pool<T> {
    /// Sparse storage; `pool[i]` holds `Some(entry)` when slot `i` is live.
    /// The u64 ID handed to callers equals the Vec index as a `u64`.
    pool: Vec<Option<PoolEntry<T>>>,

    /// Dense list of currently-live indices into `pool`.  Used for O(len)
    /// dense iteration without scanning `None` slots.
    active_indexes: Vec<usize>,

    /// Stack of recycled indices available for reuse by the next `insert`.
    free_slots: Vec<usize>,

    /// Cursor for the persistent round-robin iterators (`next_id`, `next_val`).
    /// Stored as an `AtomicUsize` so that they can advance it through
    /// a shared `&self` reference — callers that hold only a read lock on
    /// an outer container are not forced to upgrade to a write lock.
    cursor: AtomicUsize,
}

impl<T> Pool<T> {
    /// Create a new pool, pre-allocating space for `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        Pool {
            pool: Vec::with_capacity(capacity),
            active_indexes: Vec::with_capacity(capacity),
            free_slots: Vec::new(),
            cursor: AtomicUsize::new(0),
        }
    }

    /// Insert `element`, returning its stable ID.
    ///
    /// Reuses a freed slot when one is available; otherwise appends a new slot.
    pub fn insert(&mut self, element: T) -> u64 {
        // Pop from free_slots, skipping any stale entries (slot re-occupied
        // by insert_at before this insert ran).
        let idx = loop {
            match self.free_slots.pop() {
                Some(i) if self.pool[i].is_none() => break i,
                Some(_) => continue, // stale entry; skip
                None => {
                    // No free slot — extend the pool.
                    let i = self.pool.len();
                    self.pool.push(None);
                    break i;
                }
            }
        };

        let active_pos = self.active_indexes.len();
        self.pool[idx] = Some(PoolEntry {
            value: element,
            active_pos,
        });
        self.active_indexes.push(idx);
        trace!(pool_len = self.active_indexes.len(), "pool insert");
        idx as u64
    }

    /// Insert `element` at a specific `id`.
    ///
    /// Grows `pool` if `id` is beyond the current length.  If the slot was
    /// empty the index is added to `active_indexes`; if it was already
    /// occupied the element is replaced (length unchanged).
    pub fn insert_at(&mut self, element: T, id: u64) {
        let idx = id as usize;

        // Grow the pool with None slots until it covers `idx`.
        if idx >= self.pool.len() {
            self.pool.resize_with(idx + 1, || None);
        }

        match self.pool[idx].as_mut() {
            Some(entry) => {
                // Slot already live: replace value in-place, active_pos unchanged.
                entry.value = element;
            }
            None => {
                let active_pos = self.active_indexes.len();
                self.pool[idx] = Some(PoolEntry {
                    value: element,
                    active_pos,
                });
                self.active_indexes.push(idx);
            }
        }
    }

    /// Remove the element with the given `id`.  Returns `true` if an element
    /// was present, `false` if the slot was already empty.
    ///
    /// Runs in O(1): the `active_pos` back-pointer stored inside the entry
    /// lets us jump directly to the right position in `active_indexes` instead
    /// of scanning for it.
    pub fn remove(&mut self, id: u64) -> bool {
        let idx = id as usize;
        let pos = match self.pool.get(idx).and_then(|e| e.as_ref()) {
            Some(entry) => entry.active_pos,
            None => return false,
        };

        self.active_indexes.swap_remove(pos);

        // If swap_remove moved an element into `pos`, update its back-pointer.
        if pos < self.active_indexes.len() {
            let moved_slot = self.active_indexes[pos];
            self.pool[moved_slot]
                .as_mut()
                .expect("active_indexes must point to live slots")
                .active_pos = pos;
        }

        self.pool[idx] = None;
        self.free_slots.push(idx);
        true
    }

    /// Look up an element by its stable ID.
    pub fn get(&self, id: u64) -> Option<&T> {
        self.pool
            .get(id as usize)
            .and_then(|x| x.as_ref())
            .map(|entry| &entry.value)
    }

    /// Mutably look up an element by its stable ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        self.pool
            .get_mut(id as usize)
            .and_then(|x| x.as_mut())
            .map(|entry| &mut entry.value)
    }

    /// Iterate over all live elements.  No empty slots are visited.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.active_indexes.iter().map(|&i| {
            &self.pool[i]
                .as_ref()
                .expect("active_indexes must point to live slots")
                .value
        })
    }

    /// Iterate over all `(id, element)` pairs.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (u64, &T)> {
        self.active_indexes.iter().map(|&i| {
            (
                i as u64,
                &self.pool[i]
                    .as_ref()
                    .expect("active_indexes must point to live slots")
                    .value,
            )
        })
    }

    /// Iterate over the stable IDs of all live elements, analogous to
    /// `HashMap::keys()`.  Order is unspecified but consistent within a single
    /// borrow of the pool.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.active_indexes.iter().map(|&i| i as u64)
    }

    /// Persistent round-robin iterator: returns the ID of the next live
    /// element on each call, cycling back to the first when the end is
    /// reached.  Returns `None` if the pool is empty.
    ///
    /// The cursor survives across calls and is advanced atomically, so
    /// `next_id` only requires `&self` — callers that hold a shared
    /// (read) lock on an outer container are not forced to upgrade to a
    /// write lock.  If elements are removed between calls the modulo
    /// wraps the cursor back into range automatically.
    pub fn next_id(&self) -> Option<u64> {
        let n = self.active_indexes.len();
        if n == 0 {
            return None;
        }
        let pos = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
        Some(self.active_indexes[pos] as u64)
    }

    /// Round-robin iterator that returns a direct reference to the next live
    /// element, advancing the shared cursor on each call.  Shares the same
    /// cursor as `next_id`.  Returns `None` if the pool is empty.
    pub fn next_val(&self) -> Option<&T> {
        let n = self.active_indexes.len();
        if n == 0 {
            return None;
        }
        let pos = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
        let idx = self.active_indexes[pos];
        Some(
            &self.pool[idx]
                .as_ref()
                .expect("active_indexes must point to live slots")
                .value,
        )
    }

    /// Number of elements currently in the pool.
    pub fn len(&self) -> usize {
        self.active_indexes.len()
    }

    /// Current allocated capacity (elements storable without reallocation).
    pub fn capacity(&self) -> usize {
        self.pool.capacity()
    }

    /// Returns `true` if the pool contains no elements.
    pub fn is_empty(&self) -> bool {
        self.active_indexes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn test_pool() {
        let mut pool = Pool::with_capacity(10);
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());

        // Insert a few elements; IDs are assigned sequentially.
        let id0 = pool.insert(42u32);
        assert_eq!(id0, 0);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get(id0), Some(&42));
        assert_eq!(pool.get_mut(id0), Some(&mut 42));

        let id1 = pool.insert(43u32);
        assert_eq!(id1, 1);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.get(id1), Some(&43));

        let id2 = pool.insert(44u32);
        assert_eq!(id2, 2);
        assert_eq!(pool.len(), 3);

        // insert_at with an occupied ID replaces the element.
        pool.insert_at(99u32, id1);
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(id1), Some(&99));

        // insert_at with a fresh ID inserts a new element.
        pool.insert_at(55u32, 50);
        assert_eq!(pool.len(), 4);
        assert_eq!(pool.get(50), Some(&55));

        // insert returns a valid ID that resolves correctly.
        let id_next = pool.insert(77u32);
        assert_eq!(pool.get(id_next), Some(&77));

        // Remove an element; its ID no longer resolves.
        assert!(pool.remove(id0));
        assert_eq!(pool.get(id0), None);
        assert_eq!(pool.len(), 4);

        // Removing a non-existent ID returns false.
        assert!(!pool.remove(id0));

        // After removing id0, get(id1) still resolves correctly.
        assert_eq!(pool.get(id1), Some(&99));
    }

    #[test]
    fn test_pool_iter() {
        let mut pool = Pool::with_capacity(10);
        let elements = [1u32, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let ids: Vec<u64> = elements.iter().map(|&e| pool.insert(e)).collect();

        // All elements are reachable by ID.
        for (&id, &elem) in ids.iter().zip(elements.iter()) {
            assert_eq!(pool.get(id), Some(&elem));
        }

        // Iteration visits every live element exactly once (no gaps).
        let mut collected: Vec<u32> = pool.iter().copied().collect();
        collected.sort();
        let mut expected = elements.to_vec();
        expected.sort();
        assert_eq!(collected, expected);

        // Remove a few elements; iter still returns only live ones.
        pool.remove(ids[2]); // was 3
        pool.remove(ids[4]); // was 5
        pool.remove(ids[6]); // was 7

        let mut collected: Vec<u32> = pool.iter().copied().collect();
        collected.sort();
        let mut expected: Vec<u32> = elements
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2 && *i != 4 && *i != 6)
            .map(|(_, &v)| v)
            .collect();
        expected.sort();
        assert_eq!(collected, expected);
        assert_eq!(pool.len(), 7);
    }

    struct TestDropStruct<F: FnMut()> {
        drop_callback: F,
    }

    impl<F: FnMut()> Drop for TestDropStruct<F> {
        fn drop(&mut self) {
            (self.drop_callback)();
        }
    }

    #[test]
    fn test_pool_drop() {
        // All elements are dropped when the pool is dropped.
        let drop_count: RefCell<u32> = 0.into();
        let mut pool = Pool::with_capacity(10);
        (0..10).for_each(|_| {
            pool.insert(TestDropStruct {
                drop_callback: || {
                    *drop_count.borrow_mut() += 1;
                },
            });
        });
        assert_eq!(*drop_count.borrow(), 0);
        drop(pool);
        assert_eq!(*drop_count.borrow(), 10);

        // The element is dropped immediately on remove.
        let drop_count: RefCell<u32> = 0.into();
        let mut pool = Pool::with_capacity(10);
        let id = pool.insert(TestDropStruct {
            drop_callback: || {
                *drop_count.borrow_mut() += 1;
            },
        });
        assert_eq!(*drop_count.borrow(), 0);
        pool.remove(id);
        assert_eq!(*drop_count.borrow(), 1);
    }

    #[test]
    fn test_pool_grow() {
        // Start small and let the pool grow well past initial capacity.
        let mut pool = Pool::with_capacity(4);
        let ids: Vec<u64> = (0..1000u32).map(|i| pool.insert(i)).collect();
        assert_eq!(pool.len(), 1000);
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(pool.get(id), Some(&(i as u32)));
        }
    }

    #[test]
    fn test_iter_with_ids() {
        let mut pool = Pool::with_capacity(4);
        let id0 = pool.insert(10u32);
        let id1 = pool.insert(20u32);
        let id2 = pool.insert(30u32);

        let mut pairs: Vec<(u64, u32)> = pool.iter_with_ids().map(|(id, &v)| (id, v)).collect();
        pairs.sort_by_key(|&(id, _)| id);
        assert_eq!(pairs, vec![(id0, 10), (id1, 20), (id2, 30)]);

        pool.remove(id1);
        let mut pairs: Vec<(u64, u32)> = pool.iter_with_ids().map(|(id, &v)| (id, v)).collect();
        pairs.sort_by_key(|&(id, _)| id);
        assert_eq!(pairs, vec![(id0, 10), (id2, 30)]);
    }

    #[test]
    fn test_next_id_round_robin() {
        let mut pool = Pool::with_capacity(4);
        let id0 = pool.insert(10u32);
        let id1 = pool.insert(20u32);
        let id2 = pool.insert(30u32);
        let pool = pool; // rebind as immutable — next_id only needs &self

        // Collect the sequence emitted by the first full cycle.
        let first: Vec<u64> = (0..3).map(|_| pool.next_id().unwrap()).collect();
        assert_eq!(first.len(), 3);
        // Every live ID must appear exactly once per cycle.
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(sorted, vec![id0, id1, id2]);

        // The second cycle must repeat in the same order.
        let second: Vec<u64> = (0..3).map(|_| pool.next_id().unwrap()).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn test_next_id_empty() {
        let pool: Pool<u32> = Pool::with_capacity(4);
        assert_eq!(pool.next_id(), None);
    }

    #[test]
    fn test_next_val_round_robin() {
        let mut pool = Pool::with_capacity(4);
        pool.insert(10u32);
        pool.insert(20u32);
        pool.insert(30u32);
        let pool = pool; // rebind as immutable — next_val only needs &self

        // Collect the sequence emitted by the first full cycle.
        let first: Vec<u32> = (0..3).map(|_| *pool.next_val().unwrap()).collect();
        assert_eq!(first.len(), 3);
        // Every live value must appear exactly once per cycle.
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(sorted, vec![10, 20, 30]);

        // The second cycle must repeat in the same order.
        let second: Vec<u32> = (0..3).map(|_| *pool.next_val().unwrap()).collect();
        assert_eq!(first, second);
    }

    /// Removing a middle element forces a `swap_remove` that relocates the
    /// last active entry.  The back-pointer of the moved entry must be updated
    /// so subsequent removes of *that* entry are still O(1) and correct.
    #[test]
    fn test_remove_is_consistent_after_swap() {
        let mut pool = Pool::with_capacity(8);
        let id0 = pool.insert(0u32); // active_indexes: [0]
        let id1 = pool.insert(1u32); // active_indexes: [0, 1]
        let id2 = pool.insert(2u32); // active_indexes: [0, 1, 2]
        let id3 = pool.insert(3u32); // active_indexes: [0, 1, 2, 3]

        // Remove id1 (middle).  swap_remove swaps id3 into position 1.
        // active_indexes becomes [0, 3, 2].
        assert!(pool.remove(id1));
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(id1), None);

        // All survivors still reachable.
        assert_eq!(pool.get(id0), Some(&0));
        assert_eq!(pool.get(id2), Some(&2));
        assert_eq!(pool.get(id3), Some(&3));

        // iter returns exactly the three surviving values.
        let mut vals: Vec<u32> = pool.iter().copied().collect();
        vals.sort();
        assert_eq!(vals, vec![0, 2, 3]);

        // Re-insert into the freed slot, then remove it again to verify
        // active_pos is set correctly on the re-inserted entry.
        pool.insert_at(99u32, id1);
        assert_eq!(pool.len(), 4);
        assert!(pool.remove(id1));
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(id1), None);

        // Remaining entries still intact.
        assert_eq!(pool.get(id0), Some(&0));
        assert_eq!(pool.get(id2), Some(&2));
        assert_eq!(pool.get(id3), Some(&3));
    }

    /// Remove the very last element in `active_indexes`.  No back-pointer
    /// fixup is needed (the swap_remove doesn't move anything).
    #[test]
    fn test_remove_last_active_no_fixup() {
        let mut pool = Pool::with_capacity(4);
        let id0 = pool.insert(10u32);
        let id1 = pool.insert(20u32);
        let id2 = pool.insert(30u32);

        // id2 is at position 2 — the last entry; removing it needs no fixup.
        assert!(pool.remove(id2));
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.get(id2), None);
        assert_eq!(pool.get(id0), Some(&10));
        assert_eq!(pool.get(id1), Some(&20));

        // Removing the now-last element (id1) must also work.
        assert!(pool.remove(id1));
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get(id1), None);
        assert_eq!(pool.get(id0), Some(&10));
    }

    /// `insert_at` on an occupied slot must replace the value without touching
    /// `active_pos`.  A subsequent `remove` must therefore still succeed in O(1).
    #[test]
    fn test_insert_at_replace_preserves_active_pos() {
        let mut pool = Pool::with_capacity(4);
        let id0 = pool.insert(1u32);
        let id1 = pool.insert(2u32);
        let id2 = pool.insert(3u32);

        // Replace the value at id1 (occupied slot).
        pool.insert_at(99u32, id1);
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(id1), Some(&99));

        // Now remove id1; active_pos must still point to the right position.
        assert!(pool.remove(id1));
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.get(id1), None);
        assert_eq!(pool.get(id0), Some(&1));
        assert_eq!(pool.get(id2), Some(&3));
    }

    #[test]
    fn test_next_id_cursor_clamped_after_remove() {
        let mut pool = Pool::with_capacity(4);
        let id0 = pool.insert(10u32);
        let id1 = pool.insert(20u32);
        let id2 = pool.insert(30u32);

        // Advance cursor to the last position.
        assert_eq!(pool.next_id(), Some(id0));
        assert_eq!(pool.next_id(), Some(id1));
        assert_eq!(pool.next_id(), Some(id2));
        // Cursor is now at position 0 (wrapped).

        // Remove two elements so the pool has only one left; cursor must clamp.
        pool.remove(id0);
        pool.remove(id1);
        assert_eq!(pool.len(), 1);

        // next_id must still return the surviving element without panicking.
        assert_eq!(pool.next_id(), Some(id2));
        assert_eq!(pool.next_id(), Some(id2));
    }
}
