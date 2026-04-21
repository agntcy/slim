// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use tracing::trace;

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
    /// Sparse storage; `pool[i]` holds `Some(element)` when slot `i` is live.
    /// The u64 ID handed to callers equals the Vec index as a `u64`.
    pool: Vec<Option<T>>,

    /// Dense list of currently-live indices into `pool`.  Used for O(len)
    /// dense iteration without scanning `None` slots.
    active_indexes: Vec<usize>,

    /// Stack of recycled indices available for reuse by the next `insert`.
    free_slots: Vec<usize>,
}

impl<T> Pool<T> {
    /// Create a new pool, pre-allocating space for `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        Pool {
            pool: Vec::with_capacity(capacity),
            active_indexes: Vec::with_capacity(capacity),
            free_slots: Vec::new(),
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

        self.pool[idx] = Some(element);
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

        let was_empty = self.pool[idx].is_none();
        self.pool[idx] = Some(element);

        if was_empty {
            self.active_indexes.push(idx);
        }
    }

    /// Remove the element with the given `id`.  Returns `true` if an element
    /// was present, `false` if the slot was already empty.
    pub fn remove(&mut self, id: u64) -> bool {
        let idx = id as usize;

        if self.pool.get(idx).and_then(|x| x.as_ref()).is_none() {
            return false;
        }

        self.pool[idx] = None;

        // Remove from active_indexes (swap_remove is O(1) after the scan).
        if let Some(pos) = self.active_indexes.iter().position(|&i| i == idx) {
            self.active_indexes.swap_remove(pos);
        }

        self.free_slots.push(idx);
        true
    }

    /// Look up an element by its stable ID.
    pub fn get(&self, id: u64) -> Option<&T> {
        self.pool.get(id as usize).and_then(|x| x.as_ref())
    }

    /// Mutably look up an element by its stable ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        self.pool.get_mut(id as usize).and_then(|x| x.as_mut())
    }

    /// Iterate over all live elements.  No empty slots are visited.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.active_indexes.iter().map(|&i| {
            self.pool[i]
                .as_ref()
                .expect("active_indexes must point to live slots")
        })
    }

    /// Iterate over all `(id, element)` pairs.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (u64, &T)> {
        self.active_indexes.iter().map(|&i| {
            (
                i as u64,
                self.pool[i]
                    .as_ref()
                    .expect("active_indexes must point to live slots"),
            )
        })
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

}
