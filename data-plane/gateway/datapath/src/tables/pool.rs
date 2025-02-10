// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use bit_vec::BitVec;

use tracing::trace;

#[derive(Debug, Clone)]
pub struct Pool<T>
where
    T: Default + Clone,
{
    /// bitmap indicating if the pool contains an element
    bitmap: BitVec,

    /// the pool of elements
    pool: Vec<T>,

    /// the number of elements in the pool
    len: usize,

    /// the capacity of the pool
    capacity: usize,

    /// index of the bit sit with max pos
    max_set: usize,
}

impl<T> Pool<T>
where
    T: Default + Clone,
{
    /// Create a new pool with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Pool {
            bitmap: BitVec::from_elem(capacity, false),
            pool: vec![T::default(); capacity],
            len: 0,
            capacity,
            max_set: 0,
        }
    }

    /// Get the number of elements in the pool
    pub fn len(&self) -> usize {
        self.len
    }

    /// Get the capacity of the pool
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the max set index
    pub fn max_set(&self) -> usize {
        self.max_set
    }

    /// Check if the pool is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get an element from the pool
    pub fn get(&self, index: usize) -> Option<&T> {
        if self.bitmap.get(index).unwrap_or(false) {
            Some(&self.pool[index])
        } else {
            None
        }
    }

    /// Get a mutable reference to an element in the pool
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if self.bitmap.get(index).unwrap_or(false) {
            Some(&mut self.pool[index])
        } else {
            None
        }
    }

    /// Insert an element into the pool
    pub fn insert(&mut self, element: T) -> Option<usize> {
        // If length is equal to capacity, resize the pool
        if self.len == self.capacity {
            self.pool.resize(2 * self.capacity, T::default());
            self.bitmap.grow(self.capacity, false);
            self.capacity *= 2;

            trace!(
                "Resized pools to capacity: {} - {}",
                self.pool.capacity(),
                self.bitmap.capacity()
            );

            debug_assert!(self.len < self.capacity);
            debug_assert!(self.pool.capacity() >= self.capacity);
            debug_assert!(self.bitmap.capacity() >= self.capacity);
        }

        if let Some(index) = self.bitmap.iter().position(|x| !x) {
            self.pool[index] = element;
            self.bitmap.set(index, true);
            self.len += 1;

            if index > self.max_set {
                self.max_set = index;
            }

            Some(index)
        } else {
            // This should never happen
            panic!("pool is full");
        }
    }

    /// Remove an element from the pool
    pub fn remove(&mut self, index: usize) -> bool {
        if self.bitmap.get(index).unwrap_or(false) {
            self.bitmap.set(index, false);
            self.pool[index] = T::default();

            self.len -= 1;

            if index == self.max_set && index != 0 {
                // find the new max_set
                for i in (0..(index - 1)).rev() {
                    let val = self.bitmap.get(i).unwrap_or(false);
                    if val {
                        self.max_set = i;
                        break;
                    }
                }
            }

            true
        } else {
            false
        }
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_pool() {
        let mut pool = Pool::with_capacity(10);
        assert_eq!(pool.len(), 0);
        assert_eq!(pool.capacity(), 10);
        assert!(pool.is_empty());
        assert_eq!(pool.max_set(), 0);

        let element = 42;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 42));
        assert_eq!(pool.max_set(), 0);

        let element = 43;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 43));
        assert_eq!(pool.max_set(), 1);

        let element = 44;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 44));
        assert_eq!(pool.max_set(), 2);

        let element = 45;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 4);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 45));
        assert_eq!(pool.max_set(), 3);

        let element = 46;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 5);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 46));
        assert_eq!(pool.max_set(), 4);

        let element = 47;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 6);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 47));
        assert_eq!(pool.max_set(), 5);

        let element = 48;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 7);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 48));
        assert_eq!(pool.max_set(), 6);

        let element = 49;
        let index = pool.insert(element).unwrap();
        assert_eq!(pool.len(), 8);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 49));
        assert_eq!(pool.max_set(), 7);

        let current_len = pool.len();
        let mut curr_max_set = pool.max_set();

        // insert a very large number of elements in a loop to trigger resize
        for mut i in 0..1000 {
            let index = pool.insert(i).unwrap();
            assert_eq!(pool.get(index), Some(&i));
            assert_eq!(pool.get_mut(index), Some(&mut i));
            assert_eq!(pool.max_set(), curr_max_set + 1);
            curr_max_set = pool.max_set();
        }

        assert_eq!(pool.len(), current_len + 1000);

        let current_len = pool.len();
        let mut curr_max_set = pool.max_set();

        // Let's remove some random elements between 0 and 1000
        let mut removed_indexes = Vec::new();

        for i in 0..1000 {
            let pivot = rand::rng().random_range(0..1000) as usize;
            if i < pivot {
                let ret = pool.remove(i);
                assert_eq!(ret, true);

                if i == curr_max_set {
                    assert_ne!(curr_max_set, pool.max_set());
                } else {
                    assert_eq!(curr_max_set, pool.max_set());
                }
                curr_max_set = pool.max_set();

                removed_indexes.push(i);
            }
        }

        assert_eq!(pool.len(), current_len - removed_indexes.len());

        let mut curr_max_set = pool.max_set();

        // Insert new elements in the pool and check whether they are inserted in the same indexes
        for mut i in 0..removed_indexes.len() {
            let index = pool.insert(i).unwrap();
            assert_eq!(index, removed_indexes[i]);
            assert_eq!(pool.get(index), Some(&i));
            assert_eq!(pool.get_mut(index), Some(&mut i));
            if i > curr_max_set {
                assert_eq!(i, pool.max_set());
                curr_max_set = pool.max_set();
            } else {
                assert_eq!(curr_max_set, pool.max_set());
            }
        }
    }
}
