// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::RwLock;

use super::pool::Pool;

#[derive(Debug)]
pub struct ConnectionTable<T>
where
    T: Clone,
{
    /// Connection pool
    pool: RwLock<Pool<Arc<T>>>,
}

impl<T> ConnectionTable<T>
where
    T: Clone,
{
    /// Create a new connection table with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        ConnectionTable {
            pool: RwLock::new(Pool::with_capacity(capacity)),
        }
    }

    /// Add a connection to the table, returning its stable ID.
    pub fn insert(&self, connection: T) -> u64 {
        let mut pool = self.pool.write();
        pool.insert(Arc::new(connection))
    }

    /// Add a connection at a specific ID.
    pub fn insert_at(&self, connection: T, id: u64) -> bool {
        let mut pool = self.pool.write();
        pool.insert_at(Arc::new(connection), id)
    }

    /// Remove the connection with the given ID.
    pub fn remove(&self, id: u64) -> bool {
        let mut pool = self.pool.write();
        pool.remove(id)
    }

    /// Number of connections in the table.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        let pool = self.pool.read();
        pool.len()
    }

    /// Current allocated capacity.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        let pool = self.pool.read();
        pool.capacity()
    }

    /// Returns `true` if the table contains no connections.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        let pool = self.pool.read();
        pool.is_empty()
    }

    /// Look up a connection by its stable ID.
    pub fn get(&self, id: u64) -> Option<Arc<T>> {
        let pool = self.pool.read();
        pool.get(id).cloned()
    }

    /// Call `f(id, connection)` for every live connection.
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(u64, Arc<T>),
    {
        let pool = self.pool.read();
        for (id, conn_arc) in pool.iter_with_ids() {
            f(id, Arc::clone(conn_arc));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_table() {
        let table = ConnectionTable::with_capacity(10);
        assert_eq!(table.len(), 0);
        assert!(table.capacity() >= 10);
        assert!(table.is_empty());

        let connection = 10;
        let id = table.insert(connection);
        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());

        // get element from the table
        let connection_ret = table.get(id).unwrap();
        assert_eq!(*connection_ret, connection);

        // remove element from the table
        assert!(table.remove(id));

        // removing again returns false
        assert!(!table.remove(id));

        // element is no longer accessible
        assert!(table.get(id).is_none());
    }
}
