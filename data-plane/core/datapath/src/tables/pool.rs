// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use bit_vec::BitVec;

use tracing::trace;

#[derive(Debug)]
pub struct Pool<T> {
    /// bitmap indicating if the pool contains an element
    bitmap: BitVec,

    /// the pool of elements
    pool: Vec<Option<T>>,

    /// the number of elements in the pool
    len: usize,

    /// the capacity of the pool
    capacity: usize,

    /// index of the bit sit with max pos
    max_set: usize,
}

impl<T> Pool<T> {
    /// Create a new pool with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let mut pool = Vec::with_capacity(capacity);
        pool.resize_with(capacity, || None);

        Pool {
            bitmap: BitVec::from_elem(capacity, false),
            pool,
            len: 0,
            capacity,
            max_set: 0,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        Iter {
            bit_vec_iter: self.bitmap.iter(),
            pool: self,
            current_index: 0,
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
        self.pool.get(index).and_then(|slot| slot.as_ref())
    }

    /// Get a mutable reference to an element in the pool
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.pool.get_mut(index).and_then(|slot| slot.as_mut())
    }

    /// Insert an element into the pool
    pub fn insert(&mut self, element: T) -> usize {
        // If length is equal to capacity, resize the pool
        if self.len == self.capacity {
            // Resize the pool
            self.pool.resize_with(2 * self.capacity, || None);
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

        // Find the first unset bit and insert the element
        if let Some(index) = self.bitmap.iter().position(|x| !x) {
            self.insert_at(element, index)
                .then_some(true)
                .expect("insert_at failed");

            index
        } else {
            // This should never happen
            panic!("pool is full");
        }
    }

    /// Insert the element in a given position
    /// if the position does not exist the method fails
    /// return true on success
    pub fn insert_at(&mut self, element: T, index: usize) -> bool {
        if self.capacity < index {
            // position index cannot be accessed
            return false;
        }

        if !self.bitmap.get(index).unwrap_or(false) {
            // if the bit is not set, increase len
            self.len += 1;
        }

        // Mark the bit as set
        self.bitmap.set(index, true);

        // Store the new element in the pool
        self.pool[index] = Some(element);

        // If the index is greater than the max_set, update max_set
        if index > self.max_set {
            self.max_set = index;
        }

        // Return success
        true
    }

    /// Remove an element from the pool
    pub fn remove(&mut self, index: usize) -> bool {
        if self.bitmap.get(index).unwrap_or(false) {
            self.bitmap.set(index, false);

            // Remove element from the pool
            self.pool[index] = None;

            // Decrease the length of the pool
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

/// An iterator for the pool.
#[derive(Clone)]
pub struct Iter<'a, T> {
    bit_vec_iter: bit_vec::Iter<'a>,
    pool: &'a Pool<T>,
    current_index: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        // only returns the elements that are set

        // iterate until we find a true bit
        // TODO: this can be optimized a lot by skipping the elements
        // that are not set and returning the first element that is set
        for index in self.bit_vec_iter.by_ref() {
            if !index {
                // if the bit is not set, continue
                self.current_index += 1;
                continue;
            }

            // if the bit is set, return the element
            let ret = self.pool.get(self.current_index);

            // debug assert that the element is not None
            debug_assert!(ret.is_some(), "Element is None");

            // increment the current index
            self.current_index += 1;

            // return the element
            return ret;
        }

        None
    }
}

// tests
#[cfg(test)]
mod tests {
    use std::cell::RefCell;

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
        let index = pool.insert(element);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 42));
        assert_eq!(pool.max_set(), 0);

        let element = 43;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 43));
        assert_eq!(pool.max_set(), 1);

        let element = 44;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 44));
        assert_eq!(pool.max_set(), 2);

        let element = 45;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 4);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 45));
        assert_eq!(pool.max_set(), 3);

        let element = 46;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 5);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 46));
        assert_eq!(pool.max_set(), 4);

        let element = 47;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 6);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 47));
        assert_eq!(pool.max_set(), 5);

        let element = 48;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 7);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 48));
        assert_eq!(pool.max_set(), 6);

        let element = 49;
        let index = pool.insert(element);
        assert_eq!(pool.len(), 8);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 49));
        assert_eq!(pool.max_set(), 7);

        let element = 1;
        let res = pool.insert_at(element, index);
        assert!(res);
        assert_eq!(pool.len(), 8);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 1));
        assert_eq!(pool.max_set(), 7);

        let element = 56898;
        let res = pool.insert_at(element, index);
        assert!(res);
        assert_eq!(pool.len(), 8);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 56898));
        assert_eq!(pool.max_set(), 7);

        let element = 49;
        let res = pool.insert_at(element, index);
        assert!(res);
        assert_eq!(pool.len(), 8);
        assert_eq!(pool.get(index), Some(&element));
        assert_eq!(pool.get_mut(index), Some(&mut 49));
        assert_eq!(pool.max_set(), 7);

        let element = 50;
        let res = pool.insert_at(element, index + 1);
        assert!(res);
        assert_eq!(pool.len(), 9);
        assert_eq!(pool.get(index + 1), Some(&element));
        assert_eq!(pool.get_mut(index + 1), Some(&mut 50));
        assert_eq!(pool.max_set(), 8);

        let res = pool.insert_at(element, 100000000);
        assert!(!res);

        let current_len = pool.len();
        let mut curr_max_set = pool.max_set();

        // insert a very large number of elements in a loop to trigger resize
        for mut i in 0..1000 {
            let index = pool.insert(i);
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
                assert!(ret);

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
        for (mut i, idx) in removed_indexes.iter().enumerate() {
            let index = pool.insert(i);
            assert_eq!(index, *idx);
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
        // check if the drop is called for all elements in the pool at the end
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

        // check if the drop is called when an element in the pool is removed
        let drop_count: RefCell<u32> = 0.into();
        let mut pool = Pool::with_capacity(10);
        let pos = pool.insert(TestDropStruct {
            drop_callback: || {
                *drop_count.borrow_mut() += 1;
            },
        });

        assert_eq!(*drop_count.borrow(), 0);
        pool.remove(pos);
        assert_eq!(*drop_count.borrow(), 1);
    }

    #[test]
    fn test_pool_iter() {
        let mut pool = Pool::with_capacity(10);
        let elements = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        for element in &elements {
            pool.insert(*element);
        }

        let mut iter = pool.iter();
        for element in elements.iter() {
            assert_eq!(iter.next(), Some(element));
        }
        assert_eq!(iter.next(), None);

        // drop the iterator to be able to reuse the pool
        drop(iter);

        // Check that the iterator skips unset bits
        pool.remove(2);
        pool.remove(4);
        pool.remove(6);
        let mut iter = pool.iter();
        for element in elements.iter().filter(|&&x| x != 3 && x != 5 && x != 7) {
            assert_eq!(iter.next(), Some(element));
        }
        assert_eq!(iter.next(), None);
    }
}
