// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Encrypted key/value store for SLIM-level persisted state (e.g. session
//! records), used to restore sessions on restart.
//!
//! Native only in practice: the store is a plain-SQLite database whose values
//! are AES-256-GCM encrypted ([`crate::cipher`]); keys (`session:<id>`) are
//! plaintext, only values are encrypted. There is no in-memory backend — an
//! in-memory KV would not survive a restart, which defeats the purpose. On
//! `wasm32` the type is uninhabited (there is no durable store in a browser), so
//! callers simply hold `None`.

use crate::errors::PersistenceError;

#[cfg(not(target_arch = "wasm32"))]
use crate::cipher::ValueCipher;
#[cfg(not(target_arch = "wasm32"))]
use crate::group_storage::MlsEncryptionKey;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

/// Encrypted blob KV store. Backed by SQLite on native; uninhabited on `wasm32`
/// (no durable browser store), where callers hold `None` instead.
///
/// Cloning shares the same backing store (and cipher).
#[derive(Clone)]
pub enum SlimKvStore {
    #[cfg(not(target_arch = "wasm32"))]
    Sqlite(SqliteKvStore),
}

/// Opaque encrypted-SQLite backend for the KV store. Keeps the cipher out of
/// the public API surface.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct SqliteKvStore {
    inner: mls_rs_provider_sqlite::storage::SqLiteApplicationStorage,
    cipher: Arc<ValueCipher>,
}

impl std::fmt::Debug for SlimKvStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(target_arch = "wasm32"))]
        match self {
            SlimKvStore::Sqlite(_) => f.write_str("SlimKvStore::Sqlite(encrypted)"),
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = f;
            match *self {}
        }
    }
}

impl SlimKvStore {
    /// Store `value` under `key`, overwriting any existing value.
    pub fn put(&self, key: &str, value: &[u8]) -> Result<(), PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        match self {
            SlimKvStore::Sqlite(SqliteKvStore { inner, cipher }) => inner
                .insert(key, &cipher.encrypt(value)?)
                .map(|_| ())
                .map_err(|e| PersistenceError::Storage(e.to_string())),
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (key, value);
            match *self {}
        }
    }

    /// Fetch the value stored under `key`, if any.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        match self {
            SlimKvStore::Sqlite(SqliteKvStore { inner, cipher }) => match inner
                .get(key)
                .map_err(|e| PersistenceError::Storage(e.to_string()))?
            {
                Some(ct) => Ok(Some(cipher.decrypt(&ct)?)),
                None => Ok(None),
            },
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = key;
            match *self {}
        }
    }

    /// All (key, value) pairs whose key starts with `prefix`.
    pub fn list_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>, PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        match self {
            SlimKvStore::Sqlite(SqliteKvStore { inner, cipher }) => inner
                .get_by_prefix(prefix)
                .map_err(|e| PersistenceError::Storage(e.to_string()))?
                .into_iter()
                .map(|item| Ok((item.key, cipher.decrypt(&item.value)?)))
                .collect::<Result<Vec<_>, PersistenceError>>(),
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = prefix;
            match *self {}
        }
    }

    /// Delete the value stored under `key` (no-op if absent).
    pub fn delete(&self, key: &str) -> Result<(), PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        match self {
            SlimKvStore::Sqlite(SqliteKvStore { inner, .. }) => inner
                .delete(key)
                .map(|_| ())
                .map_err(|e| PersistenceError::Storage(e.to_string())),
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = key;
            match *self {}
        }
    }
}

// ---------------------------------------------------------------------------
// Native SQLite backend.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
impl SlimKvStore {
    /// Open a standalone KV store (`sessions-<hex(id)>.db`) under `dir`, with
    /// values encrypted for `identity`, using WAL journaling.
    pub fn open_sqlite(
        dir: &std::path::Path,
        identity: &str,
        key: Option<MlsEncryptionKey>,
    ) -> Result<Self, PersistenceError> {
        use mls_rs_provider_sqlite::JournalMode;

        std::fs::create_dir_all(dir)?;
        let db_path = dir.join(format!("sessions-{}.db", hex::encode(identity.as_bytes())));

        let engine = crate::sqlite_backend::open_engine(&db_path, Some(JournalMode::Wal))?;
        let inner = engine
            .application_data_storage()
            .map_err(|e| PersistenceError::Storage(e.to_string()))?;
        let cipher = ValueCipher::derive(key, identity)?;

        tracing::debug!(path = %db_path.display(), "opened encrypted SQLite session store");
        Ok(SlimKvStore::Sqlite(SqliteKvStore { inner, cipher }))
    }

    /// Construct the Sqlite variant from an already-open inner store and cipher
    /// (used by [`crate::PersistentStore::open`] to share one DB + cipher).
    pub(crate) fn from_parts(
        inner: mls_rs_provider_sqlite::storage::SqLiteApplicationStorage,
        cipher: Arc<ValueCipher>,
    ) -> Self {
        SlimKvStore::Sqlite(SqliteKvStore { inner, cipher })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn sqlite_kv_roundtrip_and_prefix() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = SlimKvStore::open_sqlite(dir.path(), "alice", None).unwrap();
            store.put("session:1", b"one").unwrap();
            store.put("session:2", b"two").unwrap();
            store.put("other:9", b"nine").unwrap();
        }

        let store = SlimKvStore::open_sqlite(dir.path(), "alice", None).unwrap();
        assert_eq!(store.get("session:1").unwrap().unwrap(), b"one");

        let mut sessions = store.list_prefix("session:").unwrap();
        sessions.sort();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].0, "session:1");
        assert_eq!(sessions[1].1, b"two".to_vec());

        store.delete("session:1").unwrap();
        assert!(store.get("session:1").unwrap().is_none());
        assert_eq!(store.list_prefix("session:").unwrap().len(), 1);
    }
}
