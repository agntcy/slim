// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Per-channel ownership: the principal that owns a channel, and so is
//! entitled to grant or revoke other participants' access to it.
//!
//! This is in-memory only for now -- it does not survive a restart. Making
//! it durable is tracked separately (channel-manager's persistence is
//! currently MLS group state/sessions only).

use std::collections::HashMap;

use tokio::sync::RwLock;

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
}

impl ChannelOwnership {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `owner` as the channel's owner, overwriting any previous
    /// value. Called once, on channel creation -- there is no ownership
    /// transfer yet (that needs the signed-grant contract).
    pub async fn set_owner(&self, channel_name: String, owner: ChannelOwner) {
        self.owners.write().await.insert(channel_name, owner);
    }

    /// Looks up a channel's owner. `None` means either the channel has no
    /// owner on record (e.g. it's config-mode, which has no creator-as-caller
    /// to default an owner from) or the channel doesn't exist.
    pub async fn get_owner(&self, channel_name: &str) -> Option<ChannelOwner> {
        self.owners.read().await.get(channel_name).cloned()
    }

    /// Removes a channel's owner record. Called on channel deletion.
    pub async fn remove_owner(&self, channel_name: &str) {
        self.owners.write().await.remove(channel_name);
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
            .await;
        assert_eq!(ownership.get_owner("a/b/c").await, Some(recorded));
    }

    #[tokio::test]
    async fn set_owner_overwrites_the_previous_value() {
        let ownership = ChannelOwnership::new();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await;
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-2"))
            .await;
        assert_eq!(ownership.get_owner("a/b/c").await, Some(owner("owner-2")));
    }

    #[tokio::test]
    async fn remove_owner_clears_the_record() {
        let ownership = ChannelOwnership::new();
        ownership
            .set_owner("a/b/c".to_string(), owner("owner-1"))
            .await;
        ownership.remove_owner("a/b/c").await;
        assert_eq!(ownership.get_owner("a/b/c").await, None);
    }

    #[tokio::test]
    async fn remove_owner_on_an_unknown_channel_is_a_no_op() {
        let ownership = ChannelOwnership::new();
        ownership.remove_owner("never-existed").await;
        assert_eq!(ownership.get_owner("never-existed").await, None);
    }
}
