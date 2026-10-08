// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel expiry: when a channel created with a TTL is due to be deleted.
//!
//! In memory, optionally written through to a `StateStore` so a channel
//! with a TTL still expires after a restart rather than becoming permanent.

use std::collections::HashMap;

use slim_persistence::PersistenceError;
use tokio::sync::RwLock;
use tracing::info;

use crate::store::StateStore;

/// Thread-safe map of channel name to the unix second it expires at.
/// Channels without a TTL have no entry.
#[derive(Default)]
pub struct ChannelExpiry {
    expires: RwLock<HashMap<String, u64>>,
    store: Option<StateStore>,
}

impl ChannelExpiry {
    /// In-memory only: expiry is lost on restart.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads expiry times from `store`, writing through to it from then on.
    /// Entries are kept only for channels `keep` accepts -- the ones
    /// actually restored -- and deleted from the store otherwise. Channels
    /// that expired while the channel manager was down are kept, so the
    /// reaper deletes them.
    pub fn load(store: StateStore, keep: impl Fn(&str) -> bool) -> Result<Self, PersistenceError> {
        let mut expires = HashMap::new();
        for (channel, expires_at) in store.load_expiries()? {
            if keep(&channel) {
                expires.insert(channel, expires_at);
            } else {
                info!(%channel, "dropping expiry record of a channel that was not restored");
                store.delete_expiry(&channel)?;
            }
        }
        Ok(Self {
            expires: RwLock::new(expires),
            store: Some(store),
        })
    }

    /// Records that the channel expires at `expires_at` (unix seconds). With
    /// a store, the record is made durable first; if that fails nothing is
    /// recorded and the error is returned.
    pub async fn set_expiry(
        &self,
        channel_name: String,
        expires_at: u64,
    ) -> Result<(), PersistenceError> {
        if let Some(store) = &self.store {
            store.put_expiry(&channel_name, expires_at)?;
        }
        self.expires.write().await.insert(channel_name, expires_at);
        Ok(())
    }

    /// When the channel expires, if it has a TTL.
    pub async fn get_expiry(&self, channel_name: &str) -> Option<u64> {
        self.expires.read().await.get(channel_name).copied()
    }

    /// Forgets a channel's expiry -- it's been deleted. The in-memory entry
    /// goes regardless; a store error is returned for the caller to report
    /// (the stale record is dropped at next startup).
    pub async fn remove_expiry(&self, channel_name: &str) -> Result<(), PersistenceError> {
        self.expires.write().await.remove(channel_name);
        match &self.store {
            Some(store) => store.delete_expiry(channel_name),
            None => Ok(()),
        }
    }

    /// Channels whose expiry time is at or before `now`.
    pub async fn expired(&self, now: u64) -> Vec<String> {
        self.expires
            .read()
            .await
            .iter()
            .filter(|(_, expires_at)| **expires_at <= now)
            .map(|(channel, _)| channel.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_only_channels_due_to_expire() {
        let expiry = ChannelExpiry::new();
        expiry
            .set_expiry("a/b/due".to_string(), 1_000)
            .await
            .unwrap();
        expiry
            .set_expiry("a/b/later".to_string(), 2_000)
            .await
            .unwrap();

        assert_eq!(expiry.expired(1_000).await, vec!["a/b/due".to_string()]);
        assert!(expiry.expired(999).await.is_empty());
    }

    #[tokio::test]
    async fn a_removed_expiry_is_no_longer_due() {
        let expiry = ChannelExpiry::new();
        expiry.set_expiry("a/b/c".to_string(), 1_000).await.unwrap();
        expiry.remove_expiry("a/b/c").await.unwrap();

        assert_eq!(expiry.get_expiry("a/b/c").await, None);
        assert!(expiry.expired(5_000).await.is_empty());
    }

    fn store(dir: &std::path::Path) -> StateStore {
        StateStore::open(dir, "org/ns/channel-manager", None).unwrap()
    }

    #[tokio::test]
    async fn an_expiry_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let before = ChannelExpiry::load(store(dir.path()), |_| true).unwrap();
        before.set_expiry("a/b/c".to_string(), 1_000).await.unwrap();

        let after = ChannelExpiry::load(store(dir.path()), |_| true).unwrap();
        assert_eq!(after.get_expiry("a/b/c").await, Some(1_000));
    }

    #[tokio::test]
    async fn loading_drops_expiry_of_channels_that_were_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        let before = ChannelExpiry::load(store(dir.path()), |_| true).unwrap();
        before
            .set_expiry("a/b/restored".to_string(), 1_000)
            .await
            .unwrap();
        before
            .set_expiry("a/b/gone".to_string(), 1_000)
            .await
            .unwrap();

        let after = ChannelExpiry::load(store(dir.path()), |c| c == "a/b/restored").unwrap();
        assert_eq!(after.get_expiry("a/b/restored").await, Some(1_000));
        assert_eq!(after.get_expiry("a/b/gone").await, None);
        let again = ChannelExpiry::load(store(dir.path()), |_| true).unwrap();
        assert_eq!(again.get_expiry("a/b/gone").await, None);
    }
}
