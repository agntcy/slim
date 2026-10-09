// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Per-channel ownership: the principal that owns a channel, and so is
//! entitled to grant or revoke other participants' access to it.
//!
//! In memory, optionally written through to a `StateStore` so ownership
//! survives a restart along with the sessions it governs.

use std::collections::HashMap;

use slim_persistence::PersistenceError;
use tokio::sync::RwLock;
use tracing::info;

use crate::store::StateStore;

/// A channel's owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelOwner {
    /// The owner's verified subject (a SPIFFE ID, a did:key, or whichever
    /// identifier the configured auth backend verifies -- see
    /// `CallerIdentity`). Used for authorization: is this caller the owner,
    /// and does a grant verify against the owner's key.
    pub subject: String,
    /// SLIM name at which the owner can be asked to approve a participant
    /// change requested without a grant. A different namespace from
    /// `subject` (routing, not identity): `None` means there is no way to
    /// reach the owner, so such requests are denied.
    pub callback_name: Option<String>,
}

/// Thread-safe map of channel name to its owner.
#[derive(Default)]
pub struct ChannelOwnership {
    owners: RwLock<HashMap<String, ChannelOwner>>,
    store: Option<StateStore>,
}

impl ChannelOwnership {
    /// In-memory only: ownership is lost on restart.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads ownership from `store`, writing through to it from then on.
    ///
    /// Owners are kept only for channels `keep` accepts -- the ones actually
    /// restored -- and the rest are deleted from the store: an owner record
    /// outliving its channel would otherwise attach itself to a different
    /// channel later created under the same name.
    pub fn load(store: StateStore, keep: impl Fn(&str) -> bool) -> Result<Self, PersistenceError> {
        let mut owners = HashMap::new();
        for (channel, owner) in store.load_owners()? {
            if keep(&channel) {
                owners.insert(channel, owner);
            } else {
                info!(%channel, "dropping owner record of a channel that was not restored");
                store.delete_owner(&channel)?;
            }
        }
        Ok(Self {
            owners: RwLock::new(owners),
            store: Some(store),
        })
    }

    /// Records `owner` as the channel's owner, overwriting any previous
    /// value. Called once, on channel creation -- there is no ownership
    /// transfer yet. With a store, the record is made durable first: if that
    /// fails, nothing is recorded and the error is returned, so the caller
    /// can refuse to leave behind a channel that would come back ownerless.
    pub async fn set_owner(
        &self,
        channel_name: String,
        owner: ChannelOwner,
    ) -> Result<(), PersistenceError> {
        if let Some(store) = &self.store {
            store.put_owner(&channel_name, &owner)?;
        }
        self.owners.write().await.insert(channel_name, owner);
        Ok(())
    }

    /// Looks up a channel's owner. `None` means either the channel has no
    /// owner on record (e.g. it's config-mode, which has no creator-as-caller
    /// to default an owner from) or the channel doesn't exist.
    pub async fn get_owner(&self, channel_name: &str) -> Option<ChannelOwner> {
        self.owners.read().await.get(channel_name).cloned()
    }

    /// Removes a channel's owner record. Called on channel deletion. The
    /// in-memory record goes regardless; a store error is returned so the
    /// caller can report it (the stale record is dropped at next startup,
    /// since its channel won't be restored).
    pub async fn remove_owner(&self, channel_name: &str) -> Result<(), PersistenceError> {
        self.owners.write().await.remove(channel_name);
        match &self.store {
            Some(store) => store.delete_owner(channel_name),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(subject: &str) -> ChannelOwner {
        ChannelOwner {
            subject: subject.to_string(),
            callback_name: None,
        }
    }

    #[tokio::test]
    async fn get_owner_is_none_for_an_unknown_channel() {
        let ownership = ChannelOwnership::new();
        assert_eq!(ownership.get_owner("a/b/c").await, None);
    }

    #[tokio::test]
    async fn set_then_get_owner_round_trips() {
        let ownership = ChannelOwnership::new();
        let recorded = ChannelOwner {
            subject: "did:key:z6Mk...".to_string(),
            callback_name: Some("org/ns/owner-shadi".to_string()),
        };
        ownership
            .set_owner("a/b/c".to_string(), recorded.clone())
            .await
            .unwrap();
        assert_eq!(ownership.get_owner("a/b/c").await, Some(recorded));
    }

    #[tokio::test]
    async fn set_owner_overwrites_the_previous_value() {
        let ownership = ChannelOwnership::new();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await
            .unwrap();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-2"))
            .await
            .unwrap();
        assert_eq!(ownership.get_owner("a/b/c").await, Some(owner("owner-2")));
    }

    #[tokio::test]
    async fn remove_owner_clears_the_record() {
        let ownership = ChannelOwnership::new();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await
            .unwrap();
        ownership.remove_owner("a/b/c").await.unwrap();
        assert_eq!(ownership.get_owner("a/b/c").await, None);
    }

    #[tokio::test]
    async fn remove_owner_on_an_unknown_channel_is_a_no_op() {
        let ownership = ChannelOwnership::new();
        ownership.remove_owner("never-existed").await.unwrap();
        assert_eq!(ownership.get_owner("never-existed").await, None);
    }

    fn store(dir: &std::path::Path) -> StateStore {
        StateStore::open(dir, "org/ns/channel-manager", None).unwrap()
    }

    #[tokio::test]
    async fn an_owner_set_with_a_store_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let ownership = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await
            .unwrap();

        let reloaded = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        assert_eq!(reloaded.get_owner("a/b/c").await, Some(owner("owner-1")));
    }

    #[tokio::test]
    async fn a_removed_owner_stays_removed_after_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let ownership = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await
            .unwrap();
        ownership.remove_owner("a/b/c").await.unwrap();

        let reloaded = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        assert_eq!(reloaded.get_owner("a/b/c").await, None);
    }

    #[tokio::test]
    async fn loading_drops_owners_of_channels_that_were_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        let ownership = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        for channel in ["a/b/restored", "a/b/gone"] {
            ownership
                .set_owner(channel.to_string(), owner("owner-1"))
                .await
                .unwrap();
        }

        let reloaded = ChannelOwnership::load(store(dir.path()), |c| c == "a/b/restored").unwrap();
        assert_eq!(
            reloaded.get_owner("a/b/restored").await,
            Some(owner("owner-1"))
        );
        assert_eq!(reloaded.get_owner("a/b/gone").await, None);
        // ...and it's gone from the store too, not just skipped this time.
        let again = ChannelOwnership::load(store(dir.path()), |_| true).unwrap();
        assert_eq!(again.get_owner("a/b/gone").await, None);
    }
}
