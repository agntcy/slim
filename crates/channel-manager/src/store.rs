// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Durable channel-manager state: channel owners, channel expiry times, and
//! consumed grant nonces.
//!
//! Kept in channel-manager's own encrypted KV store, next to (not inside)
//! the session layer's store -- same directory and passphrase, separate
//! file -- so neither has to share a database connection with the other.
//!
//! The store encrypts values but not keys, so records are keyed by a hash of
//! what they describe rather than by channel name, the same way session
//! records keep destination names out of their keys.

use std::path::Path;

use aws_lc_rs::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use slim_persistence::{MlsEncryptionKey, PersistenceError, SlimKvStore};

use crate::ownership::ChannelOwner;

const OWNER_PREFIX: &str = "owner:";
const NONCE_PREFIX: &str = "nonce:";
const EXPIRY_PREFIX: &str = "expiry:";

#[derive(Serialize, Deserialize)]
struct OwnerRecord {
    channel: String,
    subject: String,
    callback_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ExpiryRecord {
    channel: String,
    expires_at: u64,
}

#[derive(Serialize, Deserialize)]
struct NonceRecord {
    channel: String,
    nonce: String,
    not_after: u64,
}

/// A consumed grant nonce, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedNonce {
    pub channel: String,
    pub nonce: String,
    pub not_after: u64,
}

/// Channel-manager's durable state. Cloning shares the same backing store.
#[derive(Clone)]
pub struct StateStore {
    kv: SlimKvStore,
}

impl StateStore {
    /// Opens (creating if needed) the store for the channel manager named
    /// `local_name` under `dir`, encrypted with `key` -- pass the same key
    /// the session layer's store uses. `None` derives the key from
    /// `local_name`, which is public: tamper detection, no confidentiality.
    pub fn open(
        dir: &Path,
        local_name: &str,
        key: Option<MlsEncryptionKey>,
    ) -> Result<Self, PersistenceError> {
        let kv = SlimKvStore::open_sqlite(dir, &format!("channel-manager:{local_name}"), key)?;
        Ok(Self { kv })
    }

    pub fn put_owner(&self, channel: &str, owner: &ChannelOwner) -> Result<(), PersistenceError> {
        let record = OwnerRecord {
            channel: channel.to_string(),
            subject: owner.subject.clone(),
            callback_name: owner.callback_name.clone(),
        };
        self.kv.put(&owner_key(channel), &encode(&record)?)
    }

    pub fn delete_owner(&self, channel: &str) -> Result<(), PersistenceError> {
        self.kv.delete(&owner_key(channel))
    }

    pub fn load_owners(&self) -> Result<Vec<(String, ChannelOwner)>, PersistenceError> {
        self.kv
            .list_prefix(OWNER_PREFIX)?
            .into_iter()
            .map(|(_, value)| {
                let record: OwnerRecord = decode(&value)?;
                Ok((
                    record.channel,
                    ChannelOwner {
                        subject: record.subject,
                        callback_name: record.callback_name,
                    },
                ))
            })
            .collect()
    }

    pub fn put_expiry(&self, channel: &str, expires_at: u64) -> Result<(), PersistenceError> {
        let record = ExpiryRecord {
            channel: channel.to_string(),
            expires_at,
        };
        self.kv.put(&expiry_key(channel), &encode(&record)?)
    }

    pub fn delete_expiry(&self, channel: &str) -> Result<(), PersistenceError> {
        self.kv.delete(&expiry_key(channel))
    }

    pub fn load_expiries(&self) -> Result<Vec<(String, u64)>, PersistenceError> {
        self.kv
            .list_prefix(EXPIRY_PREFIX)?
            .into_iter()
            .map(|(_, value)| {
                let record: ExpiryRecord = decode(&value)?;
                Ok((record.channel, record.expires_at))
            })
            .collect()
    }

    pub fn put_nonce(&self, nonce: &ConsumedNonce) -> Result<(), PersistenceError> {
        let record = NonceRecord {
            channel: nonce.channel.clone(),
            nonce: nonce.nonce.clone(),
            not_after: nonce.not_after,
        };
        self.kv
            .put(&nonce_key(&nonce.channel, &nonce.nonce), &encode(&record)?)
    }

    pub fn delete_nonce(&self, channel: &str, nonce: &str) -> Result<(), PersistenceError> {
        self.kv.delete(&nonce_key(channel, nonce))
    }

    pub fn load_nonces(&self) -> Result<Vec<ConsumedNonce>, PersistenceError> {
        self.kv
            .list_prefix(NONCE_PREFIX)?
            .into_iter()
            .map(|(_, value)| {
                let record: NonceRecord = decode(&value)?;
                Ok(ConsumedNonce {
                    channel: record.channel,
                    nonce: record.nonce,
                    not_after: record.not_after,
                })
            })
            .collect()
    }
}

fn owner_key(channel: &str) -> String {
    format!("{OWNER_PREFIX}{}", hash_hex(channel.as_bytes()))
}

fn expiry_key(channel: &str) -> String {
    format!("{EXPIRY_PREFIX}{}", hash_hex(channel.as_bytes()))
}

fn nonce_key(channel: &str, nonce: &str) -> String {
    // NUL can't appear in a channel name, so the pair is unambiguous.
    format!(
        "{NONCE_PREFIX}{}",
        hash_hex(format!("{channel}\0{nonce}").as_bytes())
    )
}

fn hash_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>, PersistenceError> {
    serde_json::to_vec(record).map_err(|e| PersistenceError::Storage(e.to_string()))
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, PersistenceError> {
    serde_json::from_slice(bytes).map_err(|e| PersistenceError::Storage(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> StateStore {
        StateStore::open(
            dir,
            "org/ns/channel-manager",
            Some(MlsEncryptionKey::Passphrase("test-passphrase".to_string())),
        )
        .unwrap()
    }

    fn owner() -> ChannelOwner {
        ChannelOwner {
            subject: "did:key:z6Mkowner".to_string(),
            callback_name: Some("org/ns/owner-shadi".to_string()),
        }
    }

    #[test]
    fn owners_survive_reopening_the_store() {
        let dir = tempfile::tempdir().unwrap();
        open(dir.path()).put_owner("org/ns/ch1", &owner()).unwrap();

        assert_eq!(
            open(dir.path()).load_owners().unwrap(),
            vec![("org/ns/ch1".to_string(), owner())]
        );
    }

    #[test]
    fn deleting_an_owner_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.put_owner("org/ns/ch1", &owner()).unwrap();
        store.delete_owner("org/ns/ch1").unwrap();

        assert!(store.load_owners().unwrap().is_empty());
    }

    #[test]
    fn nonces_survive_reopening_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let consumed = ConsumedNonce {
            channel: "org/ns/ch1".to_string(),
            nonce: "n1".to_string(),
            not_after: 2_000,
        };
        open(dir.path()).put_nonce(&consumed).unwrap();

        assert_eq!(open(dir.path()).load_nonces().unwrap(), vec![consumed]);
    }

    #[test]
    fn expiries_survive_reopening_the_store() {
        let dir = tempfile::tempdir().unwrap();
        open(dir.path()).put_expiry("org/ns/ch1", 2_000).unwrap();
        assert_eq!(
            open(dir.path()).load_expiries().unwrap(),
            vec![("org/ns/ch1".to_string(), 2_000)]
        );

        open(dir.path()).delete_expiry("org/ns/ch1").unwrap();
        assert!(open(dir.path()).load_expiries().unwrap().is_empty());
    }

    #[test]
    fn keys_do_not_reveal_channel_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.put_owner("org/ns/secret-channel", &owner()).unwrap();
        store
            .put_nonce(&ConsumedNonce {
                channel: "org/ns/secret-channel".to_string(),
                nonce: "n1".to_string(),
                not_after: 2_000,
            })
            .unwrap();
        store.put_expiry("org/ns/secret-channel", 2_000).unwrap();

        for (key, _) in store.kv.list_prefix("").unwrap() {
            assert!(
                !key.contains("secret-channel"),
                "key leaks the channel: {key}"
            );
        }
    }
}
