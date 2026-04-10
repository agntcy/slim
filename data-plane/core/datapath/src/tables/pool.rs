// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use tracing::trace;

/// A collection that assigns each inserted element a stable `u64` ID.
///
/// IDs are assigned by a monotonically-increasing counter and are never
/// reused after removal, so a stale ID will never silently resolve to a
/// different element.  Iteration is dense — there are no empty slots to
/// skip.
#[derive(Debug)]
pub struct Pool<T> {
    /// elements keyed by their stable ID
    map: HashMap<u64, T>,

    /// next ID to hand out on insert
    next_id: u64,
}

impl<T> Pool<T> {
    /// Create a new pool, pre-allocating space for `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        Pool {
            map: HashMap::with_capacity(capacity),
            next_id: 0,
        }
    }

    /// Insert `element`, returning its stable ID.
    pub fn insert(&mut self, element: T) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.map.insert(id, element);
        trace!(pool_len = self.map.len(), "pool insert");
        id
    }

    /// Insert `element` at a specific `id`.
    ///
    /// If `id` is already occupied the element is replaced (length
    /// unchanged).  `next_id` is advanced past `id` so future calls to
    /// [`insert`] never collide with it.
    ///
    /// Always returns `true`; the signature mirrors the old API for
    /// compatibility with call-sites that check the return value.
    pub fn insert_at(&mut self, element: T, id: u64) -> bool {
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        self.map.insert(id, element);
        true
    }

    /// Remove the element with the given `id`.  Returns `true` if an
    /// element was present, `false` if the `id` was not found.
    pub fn remove(&mut self, id: u64) -> bool {
        self.map.remove(&id).is_some()
    }

    /// Look up an element by its stable ID.
    pub fn get(&self, id: u64) -> Option<&T> {
        self.map.get(&id)
    }

    /// Mutably look up an element by its stable ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        self.map.get_mut(&id)
    }

    /// Iterate over all live elements.  No empty slots are visited.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.map.values()
    }

    /// Iterate over all `(id, element)` pairs.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (u64, &T)> {
        self.map.iter().map(|(&id, v)| (id, v))
    }

    /// Number of elements currently in the pool.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Current allocated capacity (elements storable without reallocation).
    pub fn capacity(&self) -> usize {
        self.map.capacity()
    }

    /// Returns `true` if the pool contains no elements.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
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
        assert!(pool.insert_at(99u32, id1));
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(id1), Some(&99));

        // insert_at with a fresh ID inserts a new element.
        assert!(pool.insert_at(55u32, 50));
        assert_eq!(pool.len(), 4);
        assert_eq!(pool.get(50), Some(&55));

        // next_id is now at least 51 so inserts don't collide.
        let id_next = pool.insert(77u32);
        assert!(id_next >= 51);
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
    fn test_pool_ids_not_reused() {
        let mut pool = Pool::with_capacity(4);
        let a = pool.insert(1u32);
        let b = pool.insert(2u32);
        pool.remove(a);
        // The next insert must not reuse `a`.
        let c = pool.insert(3u32);
        assert_ne!(c, a);
        assert_eq!(pool.get(a), None);
        assert_eq!(pool.get(b), Some(&2));
        assert_eq!(pool.get(c), Some(&3));
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
