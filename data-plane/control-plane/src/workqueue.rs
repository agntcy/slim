// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! A work queue with the same semantics as `k8s.io/client-go/util/workqueue`:
//!
//! - **Fair**: items are processed in FIFO order.
//! - **Stingy**: an item already queued or dirty will not be added a second time;
//!   an item is never processed by two workers concurrently.
//! - **Multiple producers and consumers**: [`WorkQueue`] is `Clone + Send + Sync`.
//! - **Re-enqueue while processing**: adding an item that is currently being
//!   processed marks it *dirty*; [`done`](WorkQueue::done) automatically re-queues it.
//! - **Shutdown**: [`shutdown`](WorkQueue::shutdown) stops new adds and lets the
//!   queue drain; [`shutdown_with_drain`](WorkQueue::shutdown_with_drain) additionally
//!   waits until all in-flight items have called [`done`](WorkQueue::done).

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

// ─── Internal state ───────────────────────────────────────────────────────────

struct Inner<T> {
    queue: VecDeque<T>,
    /// Items waiting in the queue (dedup guard for queued items).
    queued: HashSet<T>,
    /// Items currently being processed.
    processing: HashSet<T>,
    /// Items re-added while being processed; re-queued automatically after done.
    dirty: HashSet<T>,
    shutdown: bool,
    /// True while shutdown_with_drain is waiting. Set to false by shutdown() to
    /// cancel the drain, matching k8s ShutDown() cancelling ShutDownWithDrain().
    drain: bool,
}

// ─── WorkQueue ────────────────────────────────────────────────────────────────

/// A fair, deduplicating, concurrency-safe work queue.
///
/// Cheaply cloneable — all clones share the same underlying queue.
pub struct WorkQueue<T> {
    inner: Arc<Mutex<Inner<T>>>,
    /// Wakes blocked pop() callers when an item is added or shutdown.
    notify: Arc<Notify>,
    /// Wakes shutdown_with_drain() when processing reaches zero or drain is cancelled.
    drain_notify: Arc<Notify>,
}

impl<T: Hash + Eq + Clone + Send + 'static> WorkQueue<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                queue: VecDeque::new(),
                queued: HashSet::new(),
                processing: HashSet::new(),
                dirty: HashSet::new(),
                shutdown: false,
                drain: false,
            })),
            notify: Arc::new(Notify::new()),
            drain_notify: Arc::new(Notify::new()),
        }
    }

    /// Add an item to the queue.
    ///
    /// - Already **queued** or **dirty** → no-op (deduplication).
    /// - Currently **processing** → mark dirty; the item will be re-queued
    ///   automatically when [`done`](Self::done) is called.
    /// - Otherwise → appended to the tail of the FIFO queue.
    /// - After **shutdown** → silently ignored.
    pub fn add(&self, item: T) {
        {
            let mut g = self.inner.lock().unwrap();
            if g.shutdown || g.queued.contains(&item) || g.dirty.contains(&item) {
                return;
            }
            if g.processing.contains(&item) {
                g.dirty.insert(item);
                return;
            }
            g.queue.push_back(item.clone());
            g.queued.insert(item);
        }
        self.notify.notify_one();
    }

    /// Wait for the next item to process.
    ///
    /// Returns `None` only after [`shutdown`](Self::shutdown) (or
    /// [`shutdown_with_drain`](Self::shutdown_with_drain)) has been called
    /// **and** the queue is empty.
    pub async fn pop(&self) -> Option<T> {
        loop {
            {
                let mut g = self.inner.lock().unwrap();
                if let Some(item) = g.queue.pop_front() {
                    g.queued.remove(&item);
                    g.processing.insert(item.clone());
                    return Some(item);
                }
                if g.shutdown {
                    return None;
                }
            }
            // Lock is dropped before the await — no mutex held across yield points.
            self.notify.notified().await;
        }
    }

    /// Mark an item as done processing.
    ///
    /// If the item was [`add`](Self::add)ed while it was being processed (dirty),
    /// it is automatically re-queued for another round.
    ///
    /// Calling `done` for an item that is not currently being processed is a no-op.
    pub fn done(&self, item: &T) {
        let (re_queued, drain_complete) = {
            let mut g = self.inner.lock().unwrap();
            if !g.processing.remove(item) {
                return;
            }
            let re_queued = if g.dirty.remove(item) {
                g.queue.push_back(item.clone());
                g.queued.insert(item.clone());
                true
            } else {
                false
            };
            // Signal drain_with_shutdown if this was the last in-flight item.
            let drain_complete = g.drain && g.processing.is_empty();
            (re_queued, drain_complete)
        };
        if re_queued {
            self.notify.notify_one();
        }
        if drain_complete {
            self.drain_notify.notify_waiters();
        }
    }

    /// Signal shutdown.
    ///
    /// New [`add`](Self::add) calls become no-ops. Existing items in the queue
    /// can still be popped and processed. Once the queue is empty,
    /// [`pop`](Self::pop) returns `None`.
    ///
    /// If [`shutdown_with_drain`](Self::shutdown_with_drain) is waiting, calling
    /// `shutdown` cancels the drain and unblocks it immediately.
    pub fn shutdown(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            g.drain = false;
            g.shutdown = true;
        }
        self.notify.notify_waiters();
        self.drain_notify.notify_waiters();
    }

    /// Shutdown and wait until all items currently being processed have called
    /// [`done`](Self::done).
    ///
    /// If [`shutdown`](Self::shutdown) was already called before this method,
    /// `drain` is already false and this returns immediately.
    pub async fn shutdown_with_drain(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            // Only enable drain if we are the ones initiating shutdown.
            // If shutdown() already ran, it set drain=false — don't override it.
            if !g.shutdown {
                g.drain = true;
                g.shutdown = true;
            }
        }
        self.notify.notify_waiters();

        loop {
            // Pre-register the listener before releasing the lock so that a
            // notify_waiters() fired between the condition check and .await is
            // not lost (tokio Notified::enable pattern).
            let notified = self.drain_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            {
                let g = self.inner.lock().unwrap();
                // Exit if drain was cancelled (by shutdown()) or no items in flight.
                if !g.drain || g.processing.is_empty() {
                    return;
                }
            }
            notified.await;
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().queue.is_empty()
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.lock().unwrap().shutdown
    }
}

impl<T: Hash + Eq + Clone + Send + 'static> Default for WorkQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for WorkQueue<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            notify: Arc::clone(&self.notify),
            drain_notify: Arc::clone(&self.drain_notify),
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    // ── Basic add / pop ────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_pop_single_item() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        assert_eq!(q.pop().await, Some("a"));
    }

    #[tokio::test]
    async fn fifo_ordering() {
        let q: WorkQueue<u32> = WorkQueue::new();
        q.add(1);
        q.add(2);
        q.add(3);
        assert_eq!(q.pop().await, Some(1));
        assert_eq!(q.pop().await, Some(2));
        assert_eq!(q.pop().await, Some(3));
    }

    // ── Deduplication ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn duplicate_add_while_queued_is_noop() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        q.add("a"); // no-op
        q.add("a"); // no-op
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop().await, Some("a"));
        assert!(q.is_empty());
    }

    // ── Dirty / re-queue ───────────────────────────────────────────────────

    #[tokio::test]
    async fn add_while_processing_requeues_after_done() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        let item = q.pop().await.unwrap();
        assert_eq!(item, "a");

        q.add("a");
        assert!(q.is_empty(), "dirty item must not appear in the queue yet");

        q.done(&item);
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop().await, Some("a"));
    }

    #[tokio::test]
    async fn multiple_dirty_adds_collapsed_to_one_requeue() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        let item = q.pop().await.unwrap();

        q.add("a"); // dirty
        q.add("a"); // already dirty — no-op
        q.add("a"); // already dirty — no-op

        q.done(&item);
        assert_eq!(q.len(), 1, "should only be one re-queued copy");
    }

    #[tokio::test]
    async fn done_without_processing_is_noop() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.done(&"ghost"); // must not panic or corrupt state
        q.add("a");
        assert_eq!(q.pop().await, Some("a"));
    }

    #[tokio::test]
    async fn no_concurrent_processing_of_same_item() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        let first = q.pop().await.unwrap();
        assert_eq!(first, "a");

        q.add("a"); // marks dirty, not queued yet
        assert!(
            timeout(Duration::from_millis(10), q.pop())
                .await
                .is_err(),
            "pop should block while item is processing"
        );

        q.done(&first);
        let second = q.pop().await.unwrap();
        assert_eq!(second, "a");
    }

    // ── Shutdown ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn shutdown_allows_draining_existing_items() {
        let q: WorkQueue<u32> = WorkQueue::new();
        q.add(1);
        q.add(2);
        q.shutdown();

        assert_eq!(q.pop().await, Some(1));
        assert_eq!(q.pop().await, Some(2));
        assert_eq!(q.pop().await, None);
    }

    #[tokio::test]
    async fn shutdown_wakes_blocked_pop() {
        let q: WorkQueue<u32> = WorkQueue::new();
        let q2 = q.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            q2.shutdown();
        });
        assert_eq!(q.pop().await, None);
    }

    #[tokio::test]
    async fn add_after_shutdown_is_noop() {
        let q: WorkQueue<u32> = WorkQueue::new();
        q.shutdown();
        q.add(1);
        assert_eq!(q.pop().await, None);
    }

    // ── ShutdownWithDrain ──────────────────────────────────────────────────

    #[tokio::test]
    async fn shutdown_with_drain_waits_for_in_flight_items() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        let item = q.pop().await.unwrap();

        let q2 = q.clone();
        let drain = tokio::spawn(async move { q2.shutdown_with_drain().await });

        // drain must not complete while "a" is still being processed.
        assert!(
            timeout(Duration::from_millis(20), drain).await.is_err(),
            "shutdown_with_drain should block while item is in flight"
        );

        // Calling done unblocks it.
        let q3 = q.clone();
        let drain2 = tokio::spawn(async move { q3.shutdown_with_drain().await });
        q.done(&item);
        timeout(Duration::from_millis(100), drain2)
            .await
            .expect("drain timed out")
            .unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_drain() {
        let q: WorkQueue<&str> = WorkQueue::new();
        q.add("a");
        let item = q.pop().await.unwrap();

        let q2 = q.clone();
        let drain = tokio::spawn(async move { q2.shutdown_with_drain().await });

        // Cancel the drain via plain shutdown().
        q.shutdown();
        timeout(Duration::from_millis(100), drain)
            .await
            .expect("shutdown did not cancel drain")
            .unwrap();

        // item never had done() called — drain exited anyway.
        drop(item);
    }

    #[tokio::test]
    async fn shutdown_with_drain_returns_immediately_when_nothing_in_flight() {
        let q: WorkQueue<u32> = WorkQueue::new();
        q.add(1);
        q.add(2);
        // Items are queued but not yet popped — nothing in flight.
        timeout(Duration::from_millis(100), q.shutdown_with_drain())
            .await
            .expect("should return immediately");
    }

    // ── Multiple producers ─────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrent_producers_deduplicate() {
        let q: WorkQueue<u32> = WorkQueue::new();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let q = q.clone();
                tokio::spawn(async move { q.add(42) })
            })
            .collect();
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(q.len(), 1);
    }

    // ── Clone shares state ─────────────────────────────────────────────────

    #[tokio::test]
    async fn clone_shares_underlying_queue() {
        let q1: WorkQueue<&str> = WorkQueue::new();
        let q2 = q1.clone();
        q1.add("x");
        assert_eq!(q2.pop().await, Some("x"));
    }
}
