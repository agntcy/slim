// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::RwLock;

use super::pool::Pool;

#[derive(Debug)]
pub struct ConnectionTable<T>
where
    T: Default + Clone,
{
    /// Connection pool
    pool: RwLock<Pool<Arc<T>>>,
}

impl<T> ConnectionTable<T>
where
    T: Default + Clone,
{
    /// Create a new connection table with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        ConnectionTable {
            pool: RwLock::new(Pool::with_capacity(capacity)),
        }
    }

    /// Add a connection to the table. This cannot fail
    pub fn insert(&self, connection: T) -> usize {
        // Get a write lock on the pool
        let mut pool = self.pool.write();
        pool.insert(Arc::new(connection))
    }

    /// Add a connection to the table on a give index
    pub fn insert_at(&self, connection: T, index: usize) -> bool {
        // Get a write lock on the pool
        let mut pool = self.pool.write();
        pool.insert_at(Arc::new(connection), index)
    }

    /// remove a connection from the table
    pub fn remove(&self, index: usize) -> bool {
        // Get a write lock on the pool
        let mut pool = self.pool.write();
        pool.remove(index)
    }

    /// Get the number of connections in the table
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        // Get a read lock on the pool
        let pool = self.pool.read();
        pool.len()
    }

    /// Get the capacity of the table
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        // Get a read lock on the pool
        let pool = self.pool.read();
        pool.capacity()
    }

    /// Check if the table is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        let pool = self.pool.read();
        pool.is_empty()
    }

    /// Get a connection from the table
    pub fn get(&self, index: usize) -> Option<Arc<T>> {
        // get a read lock on the pool
        let pool = self.pool.read();
        pool.get(index).cloned()
    }

    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(usize, Arc<T>),
    {
        let pool = self.pool.read();

        let max_index = pool.max_set();
        for idx in 0..=max_index {
            if let Some(conn_arc) = pool.get(idx) {
                f(idx, Arc::clone(conn_arc));
            }
        }
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_table() {
        let table = ConnectionTable::with_capacity(10);
        assert_eq!(table.len(), 0);
        assert_eq!(table.capacity(), 10);
        assert!(table.is_empty());

        let connection = 10;
        let index = table.insert(connection);
        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());

        // get element from the table
        let connection_ret = table.get(index).unwrap();
        assert_eq!(*connection_ret, connection);

        // remove element from the table
        assert!(table.remove(index));

        // remove an element that does not exist
        assert!(!table.remove(index));
    }
}
