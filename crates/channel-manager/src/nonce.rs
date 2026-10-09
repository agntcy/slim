// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Replay protection for signed grants: each grant's nonce is accepted once.
//!
//! In memory, optionally written through to a `StateStore` so a grant used
//! before a restart stays used after it.

use std::collections::HashMap;

use parking_lot::Mutex;
use slim_persistence::PersistenceError;
use tracing::warn;

use crate::store::{ConsumedNonce, StateStore};

/// Nonces of grants that have already been accepted, keyed by
/// `(channel, nonce)` -- so one channel's owner choosing a nonce can't burn
/// a grant for a channel they don't own.
///
/// Entries are kept until their grant's `not_after` has passed; after that
/// the grant is rejected as expired anyway, so its nonce can be forgotten.
/// They survive channel deletion: a channel recreated under the same name
/// and owner must still refuse grants already consumed against it.
#[derive(Default)]
pub struct NonceStore {
    consumed: Mutex<HashMap<(String, String), u64>>,
    store: Option<StateStore>,
}

impl NonceStore {
    /// In-memory only: consumed nonces are forgotten on restart.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads consumed nonces from `store`, writing through to it from then
    /// on. Nonces of grants already expired at `now` are dropped.
    pub fn load(store: StateStore, now: u64) -> Result<Self, PersistenceError> {
        let mut consumed = HashMap::new();
        for entry in store.load_nonces()? {
            if entry.not_after >= now {
                consumed.insert((entry.channel, entry.nonce), entry.not_after);
            } else {
                store.delete_nonce(&entry.channel, &entry.nonce)?;
            }
        }
        Ok(Self {
            consumed: Mutex::new(consumed),
            store: Some(store),
        })
    }

    /// Records the nonce of a grant for `channel` that is valid until
    /// `not_after`, returning `Ok(false)` if it was already recorded -- i.e.
    /// the grant is being replayed. Check and record happen under one lock,
    /// so two concurrent uses of the same grant can't both succeed. With a
    /// store, the nonce is made durable before it counts as recorded: if
    /// that fails, the error is returned and the grant must be refused.
    ///
    /// `now` (unix seconds) prunes nonces whose grants have expired.
    pub fn consume(
        &self,
        channel: &str,
        nonce: &str,
        not_after: u64,
        now: u64,
    ) -> Result<bool, PersistenceError> {
        let mut consumed = self.consumed.lock();
        self.prune(&mut consumed, now);

        let key = (channel.to_string(), nonce.to_string());
        if consumed.contains_key(&key) {
            return Ok(false);
        }
        if let Some(store) = &self.store {
            store.put_nonce(&ConsumedNonce {
                channel: key.0.clone(),
                nonce: key.1.clone(),
                not_after,
            })?;
        }
        consumed.insert(key, not_after);
        Ok(true)
    }

    fn prune(&self, consumed: &mut HashMap<(String, String), u64>, now: u64) {
        consumed.retain(|(channel, nonce), expires| {
            if *expires >= now {
                return true;
            }
            // A failed delete only leaves a record that the next load
            // drops anyway, as expired.
            if let Some(store) = &self.store
                && let Err(e) = store.delete_nonce(channel, nonce)
            {
                warn!(error = %e, "failed to delete an expired grant nonce");
            }
            false
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.consumed.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;
    const LATER: u64 = 2_000;

    /// A fresh random nonce, as a real grant would carry. (Literal nonces
    /// would also trip CodeQL's hard-coded cryptographic value rule.)
    fn nonce() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[test]
    fn accepts_a_nonce_once() {
        let nonces = NonceStore::new();
        let n = nonce();
        assert!(nonces.consume("org/ns/ch1", &n, LATER, NOW).unwrap());
        assert!(!nonces.consume("org/ns/ch1", &n, LATER, NOW).unwrap());
    }

    #[test]
    fn scopes_nonces_by_channel() {
        let nonces = NonceStore::new();
        let n = nonce();
        assert!(nonces.consume("org/ns/ch1", &n, LATER, NOW).unwrap());
        assert!(nonces.consume("org/ns/ch2", &n, LATER, NOW).unwrap());
    }

    #[test]
    fn keeps_a_nonce_until_its_grant_expires() {
        let nonces = NonceStore::new();
        let n = nonce();
        assert!(nonces.consume("org/ns/ch1", &n, LATER, NOW).unwrap());
        // Still within the grant's validity: replay is refused.
        assert!(!nonces.consume("org/ns/ch1", &n, LATER, LATER).unwrap());
    }

    #[test]
    fn forgets_nonces_of_expired_grants() {
        let nonces = NonceStore::new();
        assert!(nonces.consume("org/ns/ch1", &nonce(), NOW, NOW).unwrap());
        assert!(
            nonces
                .consume("org/ns/ch1", &nonce(), LATER + 10, LATER + 1)
                .unwrap()
        );
        assert_eq!(nonces.len(), 1);
    }

    fn store(dir: &std::path::Path) -> StateStore {
        StateStore::open(dir, "org/ns/channel-manager", None).unwrap()
    }

    #[test]
    fn a_consumed_nonce_stays_consumed_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let n = nonce();
        let before = NonceStore::load(store(dir.path()), NOW).unwrap();
        assert!(before.consume("org/ns/ch1", &n, LATER, NOW).unwrap());

        let after = NonceStore::load(store(dir.path()), NOW).unwrap();
        assert!(!after.consume("org/ns/ch1", &n, LATER, NOW).unwrap());
    }

    #[test]
    fn loading_drops_nonces_of_grants_that_expired_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let before = NonceStore::load(store(dir.path()), NOW).unwrap();
        assert!(before.consume("org/ns/ch1", &nonce(), LATER, NOW).unwrap());

        let after = NonceStore::load(store(dir.path()), LATER + 1).unwrap();
        assert_eq!(after.len(), 0);
        assert!(store(dir.path()).load_nonces().unwrap().is_empty());
    }
}
