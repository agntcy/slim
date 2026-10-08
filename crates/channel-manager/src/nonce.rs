// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Replay protection for signed grants: each grant's nonce is accepted once.
//!
//! In-memory only, like the rest of channel-manager's ownership state: a
//! restart forgets which nonces were consumed, so a grant used before the
//! restart is replayable after it until it expires. Making this durable is
//! tracked separately.

use std::collections::HashMap;

use parking_lot::Mutex;

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
}

impl NonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the nonce of a grant for `channel` that is valid until
    /// `not_after`, returning `false` if it was already recorded -- i.e. the
    /// grant is being replayed. Check and record happen under one lock, so
    /// two concurrent uses of the same grant can't both succeed.
    ///
    /// `now` (unix seconds) prunes nonces whose grants have expired.
    pub fn consume(&self, channel: &str, nonce: &str, not_after: u64, now: u64) -> bool {
        let mut consumed = self.consumed.lock();
        consumed.retain(|_, expires| *expires >= now);
        let key = (channel.to_string(), nonce.to_string());
        if consumed.contains_key(&key) {
            return false;
        }
        consumed.insert(key, not_after);
        true
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

    #[test]
    fn accepts_a_nonce_once() {
        let nonces = NonceStore::new();
        assert!(nonces.consume("org/ns/ch1", "n1", LATER, NOW));
        assert!(!nonces.consume("org/ns/ch1", "n1", LATER, NOW));
    }

    #[test]
    fn scopes_nonces_by_channel() {
        let nonces = NonceStore::new();
        assert!(nonces.consume("org/ns/ch1", "n1", LATER, NOW));
        assert!(nonces.consume("org/ns/ch2", "n1", LATER, NOW));
    }

    #[test]
    fn keeps_a_nonce_until_its_grant_expires() {
        let nonces = NonceStore::new();
        assert!(nonces.consume("org/ns/ch1", "n1", LATER, NOW));
        // Still within the grant's validity: replay is refused.
        assert!(!nonces.consume("org/ns/ch1", "n1", LATER, LATER));
    }

    #[test]
    fn forgets_nonces_of_expired_grants() {
        let nonces = NonceStore::new();
        assert!(nonces.consume("org/ns/ch1", "old", NOW, NOW));
        assert!(nonces.consume("org/ns/ch1", "new", LATER + 10, LATER + 1));
        assert_eq!(nonces.len(), 1);
    }
}
