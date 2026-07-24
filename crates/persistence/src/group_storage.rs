// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! MLS group state storage backends.
//!
//! `mls-rs` persists a group as an opaque snapshot through the
//! [`GroupStateStorage`] trait. [`SlimGroupStateStorage`] is a thin enum over
//! the two backends SLIM uses:
//!
//! * **In-memory** — `mls-rs`' stock provider. State is lost on restart. This
//!   is the default and the only option on `wasm32` (no filesystem).
//! * **SQLite** (native only) — a plain, statically-bundled SQLite database
//!   whose stored snapshot/epoch blobs are encrypted with AES-256-GCM
//!   (`crate::cipher`). State survives a restart, so a session can rejoin its
//!   MLS group without repeating the invite/welcome handshake.

use mls_rs::storage_provider::in_memory::InMemoryGroupStateStorage;
use mls_rs_core::group::{EpochRecord, GroupState, GroupStateStorage};
use zeroize::Zeroizing;

use crate::errors::PersistenceError;

#[cfg(not(target_arch = "wasm32"))]
use crate::cipher::ValueCipher;
#[cfg(not(target_arch = "wasm32"))]
use mls_rs_provider_sqlite::storage::SqLiteGroupStateStorage;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

/// Encryption key for the at-rest store.
///
/// A deliberately provider-agnostic key type so callers do not depend on the
/// crypto crate directly. Supplying one of these variants (via
/// [`crate::PersistenceConfig::with_key`]) is what gives the store real
/// confidentiality.
///
/// When no key is supplied the store still encrypts, but with a key derived
/// from the **public app name** rather than a secret — see
/// [`crate::PersistenceConfig::encryption_key`] for the security implications
/// of that fallback.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MlsEncryptionKey {
    /// A passphrase; a 256-bit key is derived from it via HKDF-SHA256.
    Passphrase(String),
    /// Raw 32-byte AES-256 key material.
    RawKey([u8; 32]),
}

// Never print key material.
impl std::fmt::Debug for MlsEncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MlsEncryptionKey::Passphrase(_) => f.write_str("MlsEncryptionKey::Passphrase(***)"),
            MlsEncryptionKey::RawKey(_) => f.write_str("MlsEncryptionKey::RawKey(***)"),
        }
    }
}

/// MLS group state storage: the in-memory provider, or the native SQLite
/// provider with AES-256-GCM value encryption. Cloning shares the same backing
/// store (and cipher).
#[derive(Clone)]
pub enum SlimGroupStateStorage {
    InMemory(InMemoryGroupStateStorage),
    #[cfg(not(target_arch = "wasm32"))]
    Sqlite(SqliteGroupStore),
}

/// Opaque encrypted-SQLite backend for the group store. Keeps the cipher out
/// of the public API surface.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct SqliteGroupStore {
    inner: SqLiteGroupStateStorage,
    cipher: Arc<ValueCipher>,
}

impl std::fmt::Debug for SlimGroupStateStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlimGroupStateStorage::InMemory(_) => f.write_str("SlimGroupStateStorage::InMemory"),
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(_) => {
                f.write_str("SlimGroupStateStorage::Sqlite(encrypted)")
            }
        }
    }
}

impl Default for SlimGroupStateStorage {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl SlimGroupStateStorage {
    /// A purely in-memory store (no persistence).
    pub fn in_memory() -> Self {
        SlimGroupStateStorage::InMemory(InMemoryGroupStateStorage::new())
    }

    /// True if the store persists across process restarts.
    pub fn is_persistent(&self) -> bool {
        match self {
            SlimGroupStateStorage::InMemory(_) => false,
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(_) => true,
        }
    }

    /// Delete a group's stored MLS state so it can no longer be loaded. Used
    /// when a session is permanently closed, so its MLS group is not left behind
    /// in the store. Deleting an unknown group is a no-op.
    pub fn delete_group(&self, group_id: &[u8]) -> Result<(), PersistenceError> {
        match self {
            SlimGroupStateStorage::InMemory(s) => {
                s.delete_group(group_id);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(s) => s
                .inner
                .delete_group(group_id)
                .map_err(|e| PersistenceError::Storage(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Native: plain SQLite + AES-256-GCM value encryption.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
impl SlimGroupStateStorage {
    /// Open a standalone group store (`mls-<hex(id)>.db`) under `dir`, with its
    /// snapshot/epoch values encrypted for `identity`.
    pub fn open_sqlite(
        dir: &Path,
        identity: &str,
        key: Option<MlsEncryptionKey>,
    ) -> Result<Self, PersistenceError> {
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join(format!("mls-{}.db", hex::encode(identity.as_bytes())));

        let engine = crate::sqlite_backend::open_engine(&db_path, None)?;
        let inner = engine
            .group_state_storage()
            .map_err(|e| PersistenceError::Storage(e.to_string()))?;
        let cipher = ValueCipher::derive(key, identity)?;

        tracing::debug!(path = %db_path.display(), "opened encrypted SQLite MLS group store");
        Ok(SlimGroupStateStorage::Sqlite(SqliteGroupStore {
            inner,
            cipher,
        }))
    }

    /// Construct the Sqlite variant from an already-open inner store and cipher
    /// (used by [`crate::PersistentStore::open`] to share one DB + cipher).
    pub(crate) fn from_parts(inner: SqLiteGroupStateStorage, cipher: Arc<ValueCipher>) -> Self {
        SlimGroupStateStorage::Sqlite(SqliteGroupStore { inner, cipher })
    }

    /// Group ids currently held by the store.
    pub fn stored_groups(&self) -> Result<Vec<Vec<u8>>, PersistenceError> {
        match self {
            SlimGroupStateStorage::InMemory(s) => Ok(s.stored_groups()),
            SlimGroupStateStorage::Sqlite(s) => s
                .inner
                .group_ids()
                .map_err(|e| PersistenceError::Storage(e.to_string())),
        }
    }
}

// The `GroupStateStorage` trait is sync on native and async on wasm32
// (`mls_build_async`); no method awaits, so `maybe_async` selects the shape.
#[cfg_attr(not(mls_build_async), maybe_async::must_be_sync)]
#[cfg_attr(mls_build_async, maybe_async::must_be_async)]
impl GroupStateStorage for SlimGroupStateStorage {
    type Error = PersistenceError;

    async fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        match self {
            // In-memory provider's error is `Infallible`; the empty match maps
            // the impossible error into our error type.
            SlimGroupStateStorage::InMemory(s) => s.state(group_id).await.map_err(|e| match e {}),
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(SqliteGroupStore { inner, cipher }) => {
                match inner
                    .state(group_id)
                    .await
                    .map_err(|e| PersistenceError::Storage(e.to_string()))?
                {
                    Some(ct) => Ok(Some(Zeroizing::new(cipher.decrypt(&ct)?))),
                    None => Ok(None),
                }
            }
        }
    }

    async fn epoch(
        &self,
        group_id: &[u8],
        epoch_id: u64,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        match self {
            SlimGroupStateStorage::InMemory(s) => {
                s.epoch(group_id, epoch_id).await.map_err(|e| match e {})
            }
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(SqliteGroupStore { inner, cipher }) => {
                match inner
                    .epoch(group_id, epoch_id)
                    .await
                    .map_err(|e| PersistenceError::Storage(e.to_string()))?
                {
                    Some(ct) => Ok(Some(Zeroizing::new(cipher.decrypt(&ct)?))),
                    None => Ok(None),
                }
            }
        }
    }

    async fn write(
        &mut self,
        state: GroupState,
        epoch_inserts: Vec<EpochRecord>,
        epoch_updates: Vec<EpochRecord>,
    ) -> Result<(), Self::Error> {
        match self {
            SlimGroupStateStorage::InMemory(s) => s
                .write(state, epoch_inserts, epoch_updates)
                .await
                .map_err(|e| match e {}),
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(SqliteGroupStore { inner, cipher }) => {
                // Encrypt the sensitive blobs before handing them to SQLite.
                let state = GroupState {
                    id: state.id,
                    data: Zeroizing::new(cipher.encrypt(&state.data)?),
                };
                let encrypt_epochs = |epochs: Vec<EpochRecord>| {
                    epochs
                        .into_iter()
                        .map(|e| {
                            Ok(EpochRecord::new(
                                e.id,
                                Zeroizing::new(cipher.encrypt(&e.data)?),
                            ))
                        })
                        .collect::<Result<Vec<_>, PersistenceError>>()
                };
                let epoch_inserts = encrypt_epochs(epoch_inserts)?;
                let epoch_updates = encrypt_epochs(epoch_updates)?;
                inner
                    .write(state, epoch_inserts, epoch_updates)
                    .await
                    .map_err(|e| PersistenceError::Storage(e.to_string()))
            }
        }
    }

    async fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Self::Error> {
        match self {
            SlimGroupStateStorage::InMemory(s) => {
                s.max_epoch_id(group_id).await.map_err(|e| match e {})
            }
            #[cfg(not(target_arch = "wasm32"))]
            SlimGroupStateStorage::Sqlite(SqliteGroupStore { inner, .. }) => inner
                .max_epoch_id(group_id)
                .await
                .map_err(|e| PersistenceError::Storage(e.to_string())),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn group_state(id: &[u8], data: &[u8]) -> GroupState {
        GroupState {
            id: id.to_vec(),
            data: Zeroizing::new(data.to_vec()),
        }
    }

    // On native the `GroupStateStorage` methods are sync (`is_sync`).
    #[test]
    fn in_memory_is_not_persistent() {
        let store = SlimGroupStateStorage::in_memory();
        assert!(!store.is_persistent());
    }

    #[test]
    fn sqlite_persists_and_reloads_encrypted() {
        let dir = tempfile::tempdir().unwrap();

        {
            let mut store = SlimGroupStateStorage::open_sqlite(dir.path(), "alice", None).unwrap();
            assert!(store.is_persistent());
            store
                .write(group_state(b"group-id", b"snapshot-bytes"), vec![], vec![])
                .unwrap();
            assert_eq!(store.stored_groups().unwrap(), vec![b"group-id".to_vec()]);
        }

        let reopened = SlimGroupStateStorage::open_sqlite(dir.path(), "alice", None).unwrap();
        assert_eq!(
            reopened.state(b"group-id").unwrap().unwrap().to_vec(),
            b"snapshot-bytes".to_vec()
        );
    }

    #[test]
    fn sqlite_wrong_key_cannot_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = SlimGroupStateStorage::open_sqlite(
                dir.path(),
                "alice",
                Some(MlsEncryptionKey::Passphrase("correct-horse".into())),
            )
            .unwrap();
            store
                .write(group_state(b"g", b"secret"), vec![], vec![])
                .unwrap();
        }

        // Plain SQLite opens fine, but the wrong key fails AEAD decryption.
        let store = SlimGroupStateStorage::open_sqlite(
            dir.path(),
            "alice",
            Some(MlsEncryptionKey::Passphrase("wrong-key".into())),
        )
        .unwrap();
        assert!(store.state(b"g").is_err(), "wrong key must fail to decrypt");
    }

    #[test]
    fn encryption_key_debug_is_redacted() {
        let k = MlsEncryptionKey::Passphrase("super-secret".into());
        assert_eq!(format!("{k:?}"), "MlsEncryptionKey::Passphrase(***)");
        let r = MlsEncryptionKey::RawKey([7u8; 32]);
        assert_eq!(format!("{r:?}"), "MlsEncryptionKey::RawKey(***)");
    }
}
