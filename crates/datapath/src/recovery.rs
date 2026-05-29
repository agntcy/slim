// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tracing::{debug, info};

use crate::api::ProtoName;
use crate::tables::remote_subscription_table::SubscriptionInfo;

/// All routing state preserved while waiting for a server-side peer to reconnect within the
/// recovery window.  A single [`RecoveryTable::take`] call returns everything needed to restore
/// the connection — callers never need to consult a second table.
#[derive(Debug)]
pub(crate) struct RecoveryEntry {
    /// Subscriptions in the local routing table that pointed through the dropped connection,
    /// together with their original subscription IDs so they can be restored exactly.
    pub(crate) local_subs: HashMap<ProtoName, HashSet<u64>>,
    /// Subscriptions that were forwarded *to* the remote peer on the dropped connection.
    pub(crate) remote_subs: HashSet<SubscriptionInfo>,
}

#[derive(Debug)]
struct RecoveryTableInner {
    table: RwLock<HashMap<String, RecoveryEntry>>,
    ttl: Duration,
}

/// Manages pending route-recovery state for server-side connections.
///
/// When an incoming connection drops after link negotiation completes, its routing state is
/// stored here keyed by `link_id`.  Each entry lives for at most `ttl`; it is consumed either
/// by a successful reconnect (via [`RecoveryTable::take`]) or by the TTL task spawned with
/// [`RecoveryTable::spawn_ttl_task`].
///
/// `RecoveryTable` is cheaply cloneable (Arc-backed) so that background TTL tasks can hold
/// their own handle without lifetime issues.
#[derive(Debug, Clone)]
pub(crate) struct RecoveryTable {
    inner: Arc<RecoveryTableInner>,
}

impl Default for RecoveryTable {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl RecoveryTable {
    pub(crate) fn new(ttl: Duration) -> Self {
        RecoveryTable {
            inner: Arc::new(RecoveryTableInner {
                table: RwLock::new(HashMap::new()),
                ttl,
            }),
        }
    }

    /// Store a recovery entry.  Any existing entry for `link_id` is overwritten.
    pub(crate) fn store(
        &self,
        link_id: String,
        local_subs: HashMap<ProtoName, HashSet<u64>>,
        remote_subs: HashSet<SubscriptionInfo>,
    ) {
        self.inner.table.write().insert(
            link_id,
            RecoveryEntry {
                local_subs,
                remote_subs,
            },
        );
    }

    /// Atomically remove and return the entry for `link_id`, if present.
    /// Returns `None` if the entry was already consumed (recovered or expired).
    pub(crate) fn take(&self, link_id: &str) -> Option<RecoveryEntry> {
        self.inner.table.write().remove(link_id)
    }

    pub(crate) fn ttl(&self) -> Duration {
        self.inner.ttl
    }

    /// Spawn a background task that invokes `on_expire(entry)` after `self.ttl()`, unless the
    /// drain watch fires first (in which case the task exits silently).
    ///
    /// `on_expire` is only called when the entry is still present at expiry time, i.e. recovery
    /// has not already consumed it via [`RecoveryTable::take`].
    pub(crate) fn spawn_ttl_task<F, Fut>(&self, link_id: String, drain: drain::Watch, on_expire: F)
    where
        F: FnOnce(RecoveryEntry) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let table = self.clone();
        let ttl = self.inner.ttl;
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(ttl) => {
                    if let Some(entry) = table.take(&link_id) {
                        info!(%link_id, "recovery window expired, running cleanup");
                        on_expire(entry).await;
                    }
                }
                _ = drain.signaled() => {
                    debug!(%link_id, "drain signaled, dropping pending recovery entry");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use tokio::sync::oneshot;

    use crate::api::ProtoName;
    use crate::tables::remote_subscription_table::SubscriptionInfo;

    fn empty_entry() -> (HashMap<ProtoName, HashSet<u64>>, HashSet<SubscriptionInfo>) {
        (HashMap::new(), HashSet::new())
    }

    // ── store / take ──────────────────────────────────────────────────────────

    #[test]
    fn test_take_stored_entry() {
        let t = RecoveryTable::default();
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);
        assert!(t.take("link-1").is_some());
    }

    #[test]
    fn test_take_missing_returns_none() {
        let t = RecoveryTable::default();
        assert!(t.take("nonexistent").is_none());
    }

    #[test]
    fn test_take_consumes_entry() {
        let t = RecoveryTable::default();
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);
        let _ = t.take("link-1");
        assert!(t.take("link-1").is_none(), "second take must return None");
    }

    #[test]
    fn test_store_overwrites_existing() {
        let t = RecoveryTable::default();
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);

        let mut local2: HashMap<ProtoName, HashSet<u64>> = HashMap::new();
        local2.insert(
            ProtoName::from_strings(["org", "ns", "svc"]),
            HashSet::from([1u64]),
        );
        t.store("link-1".into(), local2, HashSet::new());

        let entry = t
            .take("link-1")
            .expect("entry should exist after overwrite");
        assert_eq!(entry.local_subs.len(), 1);
    }

    #[test]
    fn test_store_multiple_distinct_keys() {
        let t = RecoveryTable::default();
        let (la, ra) = empty_entry();
        let (lb, rb) = empty_entry();
        t.store("link-a".into(), la, ra);
        t.store("link-b".into(), lb, rb);
        assert!(t.take("link-a").is_some());
        assert!(t.take("link-b").is_some());
    }

    // ── TTL configuration ─────────────────────────────────────────────────────

    #[test]
    fn test_default_ttl_is_30s() {
        assert_eq!(RecoveryTable::default().ttl(), Duration::from_secs(30));
    }

    #[test]
    fn test_custom_ttl() {
        let t = RecoveryTable::new(Duration::from_secs(5));
        assert_eq!(t.ttl(), Duration::from_secs(5));
    }

    // ── Clone shares state ────────────────────────────────────────────────────

    #[test]
    fn test_clone_shares_underlying_state() {
        let t = RecoveryTable::default();
        let clone = t.clone();

        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);

        // Clone sees the entry stored through the original handle.
        assert!(clone.take("link-1").is_some());
        // Original now sees None — entry was consumed by the clone.
        assert!(t.take("link-1").is_none());
    }

    // ── spawn_ttl_task ────────────────────────────────────────────────────────

    /// TTL expires normally: `on_expire` is called with the entry.
    #[tokio::test]
    async fn test_ttl_fires_on_expire() {
        let t = RecoveryTable::new(Duration::from_millis(50));
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);

        let (tx, rx) = oneshot::channel::<()>();
        let (_signal, watch) = drain::channel();

        t.spawn_ttl_task("link-1".into(), watch, move |_entry| async move {
            let _ = tx.send(());
        });

        tokio::time::timeout(Duration::from_millis(500), rx)
            .await
            .expect("on_expire did not fire within timeout")
            .expect("sender dropped without sending");
    }

    /// Entry consumed before TTL fires: `on_expire` must not be called.
    #[tokio::test]
    async fn test_ttl_skips_on_expire_if_entry_consumed() {
        let t = RecoveryTable::new(Duration::from_millis(50));
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);

        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();
        let (_signal, watch) = drain::channel();

        t.spawn_ttl_task("link-1".into(), watch, move |_entry| {
            let f = fired_clone.clone();
            async move { f.store(true, Ordering::SeqCst) }
        });

        // Consume the entry before the 50 ms TTL fires.
        assert!(
            t.take("link-1").is_some(),
            "entry should be present before TTL"
        );

        // Wait well past the TTL to confirm the callback is not invoked.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(
            !fired.load(Ordering::SeqCst),
            "on_expire must not fire when entry is gone"
        );
    }

    /// Drain fires before TTL: task exits silently without calling `on_expire`,
    /// and the entry remains in the table.
    #[tokio::test]
    async fn test_ttl_exits_silently_on_drain() {
        let t = RecoveryTable::new(Duration::from_secs(60)); // very long TTL
        let (local, remote) = empty_entry();
        t.store("link-1".into(), local, remote);

        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();
        let (signal, watch) = drain::channel();

        t.spawn_ttl_task("link-1".into(), watch, move |_entry| {
            let f = fired_clone.clone();
            async move { f.store(true, Ordering::SeqCst) }
        });

        // Signal drain and wait for the TTL task to acknowledge it.
        signal.drain().await;

        assert!(
            !fired.load(Ordering::SeqCst),
            "on_expire must not fire on drain"
        );
        // Entry must still be in the table — the drain path does not consume it.
        assert!(
            t.take("link-1").is_some(),
            "entry should remain in the table after drain"
        );
    }
}
