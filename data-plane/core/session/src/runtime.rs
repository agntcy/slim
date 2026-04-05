// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Minimal platform abstractions that tokio / tokio_with_wasm do not cover.
//!
//! Everything else (channels, spawn, sleep, select!, JoinHandle) is available
//! directly via `tokio::*` on both native and wasm thanks to the
//! `extern crate tokio_with_wasm as tokio` alias in lib.rs.

// ── CancellationToken ──
// tokio_util::sync::CancellationToken is not part of tokio_with_wasm,
// so we provide a unified re-export / implementation here.

#[cfg(feature = "native")]
pub use tokio_util::sync::CancellationToken;

#[cfg(all(feature = "wasm", not(feature = "native")))]
mod cancellation {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Waker;
    use parking_lot::Mutex;

    #[derive(Clone)]
    pub struct CancellationToken {
        cancelled: Arc<AtomicBool>,
        wakers: Arc<Mutex<Vec<Waker>>>,
        children: Arc<Mutex<Vec<CancellationToken>>>,
    }

    impl std::fmt::Debug for CancellationToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CancellationToken")
                .field("cancelled", &self.is_cancelled())
                .finish()
        }
    }

    impl CancellationToken {
        pub fn new() -> Self {
            Self {
                cancelled: Arc::new(AtomicBool::new(false)),
                wakers: Arc::new(Mutex::new(Vec::new())),
                children: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            let wakers = std::mem::take(&mut *self.wakers.lock());
            for waker in wakers {
                waker.wake();
            }
            let children = std::mem::take(&mut *self.children.lock());
            for child in children {
                child.cancel();
            }
        }

        pub fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        pub fn child_token(&self) -> Self {
            let child = Self::new();
            let mut children = self.children.lock();
            if self.is_cancelled() {
                drop(children);
                child.cancel();
                return child;
            }
            children.push(child.clone());
            child
        }

        fn register_waker(&self, waker: &Waker) {
            if self.is_cancelled() {
                waker.wake_by_ref();
                return;
            }
            let mut wakers = self.wakers.lock();
            if !wakers.iter().any(|w| w.will_wake(waker)) {
                wakers.push(waker.clone());
            }
        }

        pub async fn cancelled(&self) {
            use std::future::Future;
            use std::pin::Pin;
            use std::task::{Context, Poll};

            struct CancelledFuture<'a> {
                token: &'a CancellationToken,
            }

            impl Future for CancelledFuture<'_> {
                type Output = ();
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                    if self.token.is_cancelled() {
                        Poll::Ready(())
                    } else {
                        self.token.register_waker(cx.waker());
                        if self.token.is_cancelled() {
                            Poll::Ready(())
                        } else {
                            Poll::Pending
                        }
                    }
                }
            }

            CancelledFuture { token: self }.await
        }
    }

    impl Default for CancellationToken {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub use cancellation::CancellationToken;
