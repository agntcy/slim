// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The task that keeps a verifier's signing keys fresh.
//!
//! A [`RefreshTask`] is owned by the thing that holds the keys (a `KeyResolver`, or the
//! clones of an `OidcVerifier`). [`RefreshTask::start`] fetches the keys once, and fails
//! if it cannot, then spawns a task that fetches them again:
//!
//! - at 80% of the TTL, so the old keys are replaced before they expire;
//! - after a failed fetch, with a backoff, while the old keys keep being served;
//! - early when [`RefreshTask::request_refresh`] is called (a token named a key that is
//!   not cached), at most once every [`MIN_FORCED_REFRESH`]: a request that comes sooner
//!   is deferred to the end of that window, not dropped.
//!
//! The task stops when the owner is dropped: dropping the [`RefreshTask`] cancels its
//! token, which ends the timers and any request in flight. The task never owns what it
//! refreshes. It reaches the owner through what `store` captures (a `Weak`, or a cache
//! that does not own the task), so it cannot keep the owner alive.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::errors::AuthError;

/// The first retry after a failed fetch comes after this long; each failure doubles it.
const MIN_BACKOFF: Duration = Duration::from_secs(1);

/// The backoff after a failed fetch stops growing here.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A refresh asked for by [`RefreshTask::request_refresh`] runs only if the previous
/// attempt is at least this old, so tokens with made-up key ids cannot make the node
/// hammer the issuer.
pub(crate) const MIN_FORCED_REFRESH: Duration = if cfg!(test) {
    Duration::from_millis(400)
} else {
    Duration::from_secs(30)
};

/// A refresh never comes sooner than this after the previous one, whatever the TTL: a
/// TTL of zero must not turn the task into a loop that hammers the issuer.
const MIN_REFRESH_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(1)
};

/// When to refresh keys that live for `ttl`: at 80% of it.
pub(crate) fn refresh_after(ttl: Duration) -> Duration {
    ttl.mul_f32(0.8).max(MIN_REFRESH_INTERVAL)
}

/// Owns the task that refreshes one set of keys, and stops it when dropped.
#[derive(Debug)]
pub(crate) struct RefreshTask {
    cancel: CancellationToken,
    /// Cancels `cancel` when the owner of the task goes away.
    _stop: DropGuard,
    wake: Arc<Notify>,
    /// The running task. The lock also serializes concurrent calls to `start`.
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RefreshTask {
    pub(crate) fn new() -> Self {
        let cancel = CancellationToken::new();
        Self {
            _stop: cancel.clone().drop_guard(),
            cancel,
            wake: Arc::new(Notify::new()),
            worker: Mutex::new(None),
        }
    }

    /// Fetch the keys now and keep them fresh from then on.
    ///
    /// Returns once the first keys are stored, and fails if they cannot be fetched (a
    /// failed fetch starts no task, so a later call tries again). Does nothing if the
    /// task is already running; a task that ended (it panicked) is started again.
    ///
    /// `fetch` gets the keys. It must not own what they are stored in: it runs without
    /// a hold on the owner, so that dropping the owner can cancel it. `store` saves what
    /// `fetch` returned and returns `false` once the owner is gone, which ends the task.
    /// `ttl` is how long stored keys stay valid.
    pub(crate) async fn start<T, Fetch, FetchFut, Store>(
        &self,
        ttl: Duration,
        fetch: Fetch,
        store: Store,
    ) -> Result<(), AuthError>
    where
        T: Send + 'static,
        Fetch: Fn() -> FetchFut + Send + 'static,
        FetchFut: Future<Output = Result<T, AuthError>> + Send + 'static,
        Store: Fn(T) -> bool + Send + 'static,
    {
        let mut worker = self.worker.lock().await;
        if worker.as_ref().is_some_and(|task| !task.is_finished()) {
            return Ok(());
        }

        if !store(fetch().await?) {
            return Ok(());
        }
        *worker = Some(tokio::spawn(refresh_loop(
            self.cancel.clone(),
            self.wake.clone(),
            ttl,
            fetch,
            store,
        )));
        Ok(())
    }

    /// Ask for a refresh before the timer: a token named a key the cache lacks. Does
    /// nothing if the task is not running. If the task attempted a fetch less than
    /// [`MIN_FORCED_REFRESH`] ago, the refresh waits until that much time has passed.
    pub(crate) fn request_refresh(&self) {
        self.wake.notify_one();
    }

    /// Whether [`RefreshTask::start`] succeeded and the task is still running.
    pub(crate) fn is_running(&self) -> bool {
        self.worker
            .try_lock()
            .map(|worker| worker.as_ref().is_some_and(|task| !task.is_finished()))
            // Locked: `start` is in progress, which is as good as running.
            .unwrap_or(true)
    }
}

async fn refresh_loop<T, Fetch, FetchFut, Store>(
    cancel: CancellationToken,
    wake: Arc<Notify>,
    ttl: Duration,
    fetch: Fetch,
    store: Store,
) where
    Fetch: Fn() -> FetchFut,
    FetchFut: Future<Output = Result<T, AuthError>>,
    Store: Fn(T) -> bool,
{
    let mut backoff = MIN_BACKOFF;
    // The first keys were fetched by `start`, just now.
    let mut last_attempt = Instant::now();
    let mut due = last_attempt + refresh_after(ttl);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep_until(due) => {}
            _ = wake.notified() => {
                let earliest = last_attempt + MIN_FORCED_REFRESH;
                if Instant::now() < earliest {
                    // Too soon: do not drop the request, bring the next refresh
                    // forward to the earliest moment it is allowed.
                    due = due.min(earliest);
                    continue;
                }
            }
        }

        last_attempt = Instant::now();
        let fetched = tokio::select! {
            _ = cancel.cancelled() => return,
            fetched = fetch() => fetched,
        };
        match fetched {
            Ok(keys) => {
                if !store(keys) {
                    return;
                }
                backoff = MIN_BACKOFF;
                due = Instant::now() + refresh_after(ttl);
            }
            Err(error) => {
                tracing::warn!(%error, retry_in = ?backoff, "refreshing the signing keys failed");
                due = Instant::now() + backoff;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A fetch that counts its calls and fails while `failing` is set.
    struct Source {
        calls: AtomicUsize,
        failing: std::sync::atomic::AtomicBool,
        stored: AtomicUsize,
    }

    impl Source {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                failing: std::sync::atomic::AtomicBool::new(false),
                stored: AtomicUsize::new(0),
            })
        }

        async fn start(
            self: &Arc<Self>,
            task: &RefreshTask,
            ttl: Duration,
        ) -> Result<(), AuthError> {
            let (fetch_from, store_in) = (self.clone(), self.clone());
            task.start(
                ttl,
                move || {
                    let source = fetch_from.clone();
                    async move {
                        source.calls.fetch_add(1, Ordering::SeqCst);
                        if source.failing.load(Ordering::SeqCst) {
                            Err(AuthError::WouldBlockOn)
                        } else {
                            Ok(())
                        }
                    }
                },
                move |()| {
                    store_in.stored.fetch_add(1, Ordering::SeqCst);
                    true
                },
            )
            .await
        }
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// `start` waits for the first keys, and fails if it cannot get them.
    #[tokio::test]
    async fn test_start_fails_when_the_first_fetch_fails() {
        let task = RefreshTask::new();
        let source = Source::new();
        source.failing.store(true, Ordering::SeqCst);

        assert!(source.start(&task, Duration::from_secs(60)).await.is_err());
        assert!(!task.is_running());

        // A failed start left nothing behind: the next call tries again.
        source.failing.store(false, Ordering::SeqCst);
        source.start(&task, Duration::from_secs(60)).await.unwrap();
        assert!(task.is_running());
        assert_eq!(source.stored.load(Ordering::SeqCst), 1);
    }

    /// Keys are refreshed on their own, at 80% of the TTL, with nobody asking.
    #[tokio::test]
    async fn test_keys_are_refreshed_before_the_ttl_runs_out() {
        let task = RefreshTask::new();
        let source = Source::new();
        source
            .start(&task, Duration::from_millis(250))
            .await
            .unwrap();

        until("two refreshes", || {
            source.stored.load(Ordering::SeqCst) >= 3
        })
        .await;
    }

    /// `start` on a running task does not start a second one.
    #[tokio::test]
    async fn test_start_is_idempotent() {
        let task = RefreshTask::new();
        let source = Source::new();
        source.start(&task, Duration::from_secs(60)).await.unwrap();
        source.start(&task, Duration::from_secs(60)).await.unwrap();
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }

    /// A failed refresh is retried after a backoff, and the task goes on after it.
    #[tokio::test]
    async fn test_a_failed_refresh_is_retried() {
        let task = RefreshTask::new();
        let source = Source::new();
        source
            .start(&task, Duration::from_millis(100))
            .await
            .unwrap();

        source.failing.store(true, Ordering::SeqCst);
        until("a failed attempt", || {
            source.calls.load(Ordering::SeqCst) >= 2
        })
        .await;
        let stored = source.stored.load(Ordering::SeqCst);

        source.failing.store(false, Ordering::SeqCst);
        until("a successful retry", || {
            source.stored.load(Ordering::SeqCst) > stored
        })
        .await;
        assert!(task.is_running());
    }

    /// Dropping the owner ends the task, even in the middle of a fetch.
    #[tokio::test]
    async fn test_dropping_the_owner_cancels_the_task_and_its_fetch() {
        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let task = RefreshTask::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let cancelled_fetches = Arc::new(AtomicUsize::new(0));
        let (fetch_calls, fetch_cancelled) = (calls.clone(), cancelled_fetches.clone());
        task.start(
            Duration::from_millis(100),
            move || {
                let n = fetch_calls.fetch_add(1, Ordering::SeqCst);
                let guard = Dropped(fetch_cancelled.clone());
                async move {
                    let _guard = guard;
                    if n > 0 {
                        // Every refresh hangs, as an issuer that stopped answering.
                        std::future::pending::<()>().await;
                    }
                    Ok(())
                }
            },
            |()| true,
        )
        .await
        .unwrap();

        until("a hung refresh", || calls.load(Ordering::SeqCst) >= 2).await;
        drop(task);
        until("the fetch to be dropped", || {
            cancelled_fetches.load(Ordering::SeqCst) >= 2
        })
        .await;

        // And nothing runs afterwards.
        let after = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(calls.load(Ordering::SeqCst), after);
    }

    /// The task ends when `store` says the owner is gone.
    #[tokio::test]
    async fn test_the_task_ends_when_the_owner_is_gone() {
        let task = RefreshTask::new();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let stores = Arc::new(AtomicUsize::new(0));
        let (store_alive, store_count) = (alive.clone(), stores.clone());
        task.start(
            Duration::from_millis(100),
            || async { Ok(()) },
            move |()| {
                store_count.fetch_add(1, Ordering::SeqCst);
                store_alive.load(Ordering::SeqCst)
            },
        )
        .await
        .unwrap();

        alive.store(false, Ordering::SeqCst);
        until("the task to end", || !task.is_running()).await;
    }

    /// A requested refresh comes before the timer, but not within MIN_FORCED_REFRESH of
    /// the previous attempt: one that comes sooner is deferred to the end of that window,
    /// once, however often it is asked for.
    #[tokio::test]
    async fn test_a_requested_refresh_is_deferred_to_the_end_of_the_window() {
        let task = RefreshTask::new();
        let source = Source::new();
        source
            .start(&task, Duration::from_secs(3600))
            .await
            .unwrap();

        for _ in 0..5 {
            task.request_refresh();
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            1,
            "not within the window"
        );

        until("the deferred refresh", || {
            source.calls.load(Ordering::SeqCst) >= 2
        })
        .await;
        tokio::time::sleep(MIN_FORCED_REFRESH / 2).await;
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            2,
            "once, not once per request"
        );
    }

    /// Once the window has passed, a request is served at once, long before the timer.
    #[tokio::test]
    async fn test_a_requested_refresh_after_the_window_is_served_at_once() {
        let task = RefreshTask::new();
        let source = Source::new();
        source
            .start(&task, Duration::from_secs(3600))
            .await
            .unwrap();

        tokio::time::sleep(MIN_FORCED_REFRESH + Duration::from_millis(50)).await;
        task.request_refresh();
        until("the requested refresh", || {
            source.calls.load(Ordering::SeqCst) >= 2
        })
        .await;
    }

    /// With time paused: keys are refreshed at 80% of the TTL, not sooner, not later.
    #[tokio::test(start_paused = true)]
    async fn test_refresh_comes_at_eighty_percent_of_the_ttl() {
        let task = RefreshTask::new();
        let source = Source::new();
        source.start(&task, Duration::from_secs(100)).await.unwrap();
        advance(Duration::ZERO).await; // let the task begin its schedule
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);

        advance(Duration::from_secs(79)).await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 1, "at 79%");
        advance(Duration::from_secs(2)).await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 2, "past 80%");

        // And the schedule starts over from that refresh.
        advance(Duration::from_secs(79)).await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        advance(Duration::from_secs(2)).await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 3);
    }

    /// With time paused: a failing refresh is retried after 1 s, 2 s, 4 s, ... and the wait
    /// stops growing at 30 s; a success resets it.
    #[tokio::test(start_paused = true)]
    async fn test_the_backoff_doubles_up_to_its_cap_and_resets_on_success() {
        let task = RefreshTask::new();
        let source = Source::new();
        source.start(&task, Duration::from_secs(10)).await.unwrap();
        advance(Duration::ZERO).await; // let the task begin its schedule
        source.failing.store(true, Ordering::SeqCst);

        // The refresh is due at 8 s; each failure then waits 1, 2, 4, 8, 16, 30, 30 s.
        advance(Duration::from_secs(9)).await;
        let mut calls = source.calls.load(Ordering::SeqCst);
        assert_eq!(calls, 2, "the first failed refresh");
        for wait in [1u64, 2, 4, 8, 16, 30, 30] {
            advance(Duration::from_secs(wait - 1)).await;
            assert_eq!(
                source.calls.load(Ordering::SeqCst),
                calls,
                "before {wait} s"
            );
            advance(Duration::from_secs(2)).await;
            calls += 1;
            assert_eq!(source.calls.load(Ordering::SeqCst), calls, "after {wait} s");
        }

        // A success resets the backoff: the next failure waits 1 s again.
        source.failing.store(false, Ordering::SeqCst);
        advance(Duration::from_secs(31)).await;
        let stored = source.stored.load(Ordering::SeqCst);
        assert!(stored >= 2, "the recovery is stored");
        source.failing.store(true, Ordering::SeqCst);
        advance(Duration::from_secs(9)).await;
        calls = source.calls.load(Ordering::SeqCst);
        advance(Duration::from_secs(2)).await;
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            calls + 1,
            "1 s after the first failure"
        );
    }

    /// Move the paused clock forward and let the task run.
    async fn advance(by: Duration) {
        tokio::time::advance(by).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }
}
