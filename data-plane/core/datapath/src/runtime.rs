// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Platform abstractions used by the data plane.
//!
//! On native builds, [`CancellationToken`] is re-exported from
//! `tokio_util::sync` and the `drain` crate is used directly for graceful
//! shutdown. On wasm builds, `tokio_util` is unavailable and the `drain` crate
//! pulls in non-portable platform code, so we provide:
//!
//! * A small `CancellationToken` shim mirroring the `tokio_util` API surface
//!   used by the data plane.
//! * A drain compatibility layer (`Signal`, `Watch`, `channel`) backed by the
//!   cancellation token. The wasm shim is API-compatible with `drain` for the
//!   subset that the data plane uses (`Watch::signaled`, `Signal::drain`),
//!   but does not wait for outstanding watchers — it simply cancels.

#[cfg(feature = "native")]
pub use tokio_util::sync::CancellationToken;

#[cfg(feature = "native")]
pub use drain::{Signal as DrainSignal, Watch as DrainWatch};

#[cfg(feature = "native")]
pub fn drain_channel() -> (DrainSignal, DrainWatch) {
    drain::channel()
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
mod cancellation {
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Waker;

    /// Browser-friendly subset of the `tokio_util::sync::CancellationToken`
    /// API used by the data plane. WASM is single-threaded so the wakers
    /// stored here are only ever polled on the JS event loop.
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

#[cfg(all(feature = "wasm", not(feature = "native")))]
mod drain_shim {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::CancellationToken;

    /// Browser stand-in for `drain::Signal`. Cancelling this signal cancels
    /// the underlying token; the `drain` future returns immediately because
    /// there are no native worker threads to wait on.
    #[derive(Debug)]
    pub struct DrainSignal {
        token: CancellationToken,
    }

    impl DrainSignal {
        pub async fn drain(self) {
            self.token.cancel();
        }
    }

    /// Browser stand-in for `drain::Watch`. `signaled()` resolves once the
    /// associated `DrainSignal` has been drained.
    #[derive(Debug, Clone)]
    pub struct DrainWatch {
        token: CancellationToken,
    }

    impl DrainWatch {
        pub fn signaled(self) -> DrainSignaled {
            DrainSignaled { token: self.token }
        }
    }

    /// Future returned by `DrainWatch::signaled()`.
    pub struct DrainSignaled {
        token: CancellationToken,
    }

    impl Future for DrainSignaled {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            // Reuse the cancellation future logic by polling a fresh
            // `cancelled()` future on each call. Because WASM is single
            // threaded, this allocation is cheap.
            let mut fut = Box::pin(self.token.cancelled());
            fut.as_mut().poll(cx)
        }
    }

    pub fn drain_channel() -> (DrainSignal, DrainWatch) {
        let token = CancellationToken::new();
        (
            DrainSignal {
                token: token.clone(),
            },
            DrainWatch { token },
        )
    }
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub use drain_shim::{DrainSignal, DrainWatch, drain_channel};
