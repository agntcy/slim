// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use super::pool::Pool;

/// A connection table that provides lock-free reads via `ArcSwap`.
///
/// Concurrency design:
///   - Hot read path (`get`, `for_each`, `len`, `is_empty`): lock-free via
///     `ArcSwap::load()`.  A single atomic pointer load gives callers an
///     immutable snapshot of the pool.
///   - Write path (`insert`, `insert_at`, `remove`): serialised by
///     `write_lock`.  Writers load the current snapshot, clone the pool,
///     apply the change, then atomically store the new `Arc`.  No write
///     lock is ever held during reads.
///
/// Connections are stored as `Arc<T>` so that pool clones (made on every
/// write) only bump refcounts rather than deep-copying each connection.
/// `get()` returns `Option<Arc<T>>`; callers can auto-deref into `T`.
#[derive(Debug)]
pub struct ConnectionTable<T>
where
    T: Clone,
{
    pool: ArcSwap<Pool<Arc<T>>>,
    write_lock: Mutex<()>,
}

impl<T> ConnectionTable<T>
where
    T: Clone,
{
    /// Create a new connection table with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        ConnectionTable {
            pool: ArcSwap::from_pointee(Pool::with_capacity(capacity)),
            write_lock: Mutex::new(()),
        }
    }

    /// Add a connection to the table, returning its stable ID.
    pub fn insert(&self, connection: T) -> u64 {
        let _guard = self.write_lock.lock();
        let mut pool = (**self.pool.load()).clone();
        let id = pool.insert(Arc::new(connection));
        self.pool.store(Arc::new(pool));
        id
    }

    /// Add a connection at a specific ID.
    pub fn insert_at(&self, connection: T, id: u64) {
        let _guard = self.write_lock.lock();
        let mut pool = (**self.pool.load()).clone();
        pool.insert_at(Arc::new(connection), id);
        self.pool.store(Arc::new(pool));
    }

    /// Remove the connection with the given ID.
    pub fn remove(&self, id: u64) -> bool {
        let _guard = self.write_lock.lock();
        let mut pool = (**self.pool.load()).clone();
        let existed = pool.remove(id);
        self.pool.store(Arc::new(pool));
        existed
    }

    /// Number of connections in the table.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.pool.load().len()
    }

    /// Current allocated capacity.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.pool.load().capacity()
    }

    /// Returns `true` if the table contains no connections.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.pool.load().is_empty()
    }

    /// Look up a connection by its stable ID.
    pub fn get(&self, id: u64) -> Option<Arc<T>> {
        self.pool.load().get(id).cloned()
    }

    /// Call `f(id, connection)` for every live connection.
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(u64, Arc<T>),
    {
        let pool = self.pool.load();
        for (id, conn) in pool.iter_with_ids() {
            f(id, conn.clone());
        }
    }

    /// Mutate a connection in-place (copy-on-write).
    ///
    /// Clones the pool, applies `f` to the entry, and replaces the pool atomically.
    /// Returns `true` if the connection was found and updated.
    pub fn update<F>(&self, id: u64, f: F) -> bool
    where
        F: FnOnce(&mut T),
    {
        let _guard = self.write_lock.lock();
        let mut pool = (**self.pool.load()).clone();
        if let Some(entry) = pool.get_mut(id) {
            let conn = Arc::make_mut(entry);
            f(conn);
            self.pool.store(Arc::new(pool));
            true
        } else {
            false
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
