// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Unified persistent store: one encrypted database file holding both the MLS
//! group state and the SLIM session records for an identity.
//!
//! [`PersistentStore::open`] builds a single plain SQLite database
//! (`slim-<hex(identity)>.db`) whose stored values are AES-256-GCM encrypted
//! ([`crate::cipher`]), and returns both a [`SlimGroupStateStorage`] (for the
//! MLS layer) and a [`SlimKvStore`] (for session records) over it, sharing one
//! cipher. The two use separate tables in the same file, so one key, one file,
//! and one lifecycle cover all of a session's restorable state.

// Used by `PersistenceConfig` on all targets.
use crate::group_storage::MlsEncryptionKey;

// The store opener is native-only (durable persistence needs a filesystem).
#[cfg(not(target_arch = "wasm32"))]
use crate::errors::PersistenceError;
#[cfg(not(target_arch = "wasm32"))]
use crate::group_storage::SlimGroupStateStorage;
#[cfg(not(target_arch = "wasm32"))]
use crate::kv_store::SlimKvStore;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// Where and how a session's state is persisted.
///
/// Serde-(de)serializable so it can be embedded as a `persistence:` section of
/// the app/node config in addition to being constructed programmatically.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistenceConfig {
    /// Directory holding the encrypted store. One database file per endpoint
    /// identity is kept here.
    pub path: std::path::PathBuf,

    /// Optional explicit encryption key. When `None`, the key is derived from
    /// the endpoint identity — convenient and stable across restarts, but only
    /// as secret as the identity id; supply an explicit key for strong
    /// confidentiality.
    pub encryption_key: Option<MlsEncryptionKey>,
}

impl PersistenceConfig {
    /// Persist under `path`, deriving the encryption key from the identity.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            encryption_key: None,
        }
    }

    /// Persist under `path` using an explicit encryption key.
    pub fn with_key(path: impl Into<std::path::PathBuf>, key: MlsEncryptionKey) -> Self {
        Self {
            path: path.into(),
            encryption_key: Some(key),
        }
    }
}

/// Opener for the unified MLS + session-record store.
///
/// Native only: durable persistence requires a filesystem. On `wasm32` there is
/// no durable store — a page reload discards the WebAssembly instance's memory —
/// so persistence is not offered there at all (the session layer treats it as
/// disabled).
#[cfg(not(target_arch = "wasm32"))]
pub struct PersistentStore;

#[cfg(not(target_arch = "wasm32"))]
impl PersistentStore {
    /// Open the encrypted store identified by `store_key` under `dir`, returning
    /// the MLS group-state handle and the session-record KV handle backed by the
    /// same database file.
    ///
    /// `store_key` must be **stable across restarts** for the same logical store
    /// (e.g. derived from the app name), since it names the file and, absent an
    /// explicit `key`, seeds the encryption key.
    pub fn open(
        dir: &Path,
        store_key: &str,
        key: Option<MlsEncryptionKey>,
    ) -> Result<(SlimGroupStateStorage, SlimKvStore), PersistenceError> {
        use mls_rs_provider_sqlite::JournalMode;

        std::fs::create_dir_all(dir)?;
        let db_path = dir.join(format!("slim-{}.db", hex::encode(store_key.as_bytes())));

        // One engine, one file. The first handle created runs the schema setup
        // (all tables at once); the second sees it and skips — no races.
        let engine = crate::sqlite_backend::open_engine(&db_path, Some(JournalMode::Wal))?;

        let group = engine
            .group_state_storage()
            .map_err(|e| PersistenceError::Storage(e.to_string()))?;
        let kv = engine
            .application_data_storage()
            .map_err(|e| PersistenceError::Storage(e.to_string()))?;

        // One cipher shared by both handles (same key derivation).
        let cipher = crate::cipher::ValueCipher::derive(key, store_key)?;

        tracing::debug!(path = %db_path.display(), "opened unified encrypted SLIM store");
        Ok((
            SlimGroupStateStorage::from_parts(group, cipher.clone()),
            SlimKvStore::from_parts(kv, cipher),
        ))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use mls_rs_core::group::{GroupState, GroupStateStorage};
    use zeroize::Zeroizing;

    #[test]
    fn unified_store_shares_one_file() {
        let dir = tempfile::tempdir().unwrap();

        {
            let (mut group, kv) = PersistentStore::open(dir.path(), "alice", None).unwrap();
            group
                .write(
                    GroupState {
                        id: b"g".to_vec(),
                        data: Zeroizing::new(b"snap".to_vec()),
                    },
                    vec![],
                    vec![],
                )
                .unwrap();
            kv.put("session:1", b"rec").unwrap();
        }

        // Exactly one database file exists, holding both kinds of state.
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".db"))
            .collect();
        assert_eq!(files.len(), 1, "expected a single db file, got {files:?}");

        let (group, kv) = PersistentStore::open(dir.path(), "alice", None).unwrap();
        assert_eq!(
            group.state(b"g").unwrap().unwrap().to_vec(),
            b"snap".to_vec()
        );
        assert_eq!(kv.get("session:1").unwrap().unwrap(), b"rec".to_vec());
    }
}
