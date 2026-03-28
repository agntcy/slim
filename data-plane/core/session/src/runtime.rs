// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Platform-agnostic async runtime abstractions.
//!
//! On native targets, delegates to tokio.
//! On WASM targets, delegates to futures-channel and wasm-bindgen-futures.

// ── Channel types ──

#[cfg(feature = "native")]
pub mod channel {
    pub mod mpsc {
        pub use tokio::sync::mpsc::{
            Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
        };
    }
    pub mod oneshot {
        pub use tokio::sync::oneshot::{Receiver, Sender, channel};
    }
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub mod channel {
    pub mod mpsc {
        use std::pin::Pin;
        use std::task::{Context, Poll};

        // ── Error type ──

        #[derive(Debug)]
        pub struct SendError<T>(pub T);

        impl<T> std::fmt::Display for SendError<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "channel closed")
            }
        }

        impl<T: std::fmt::Debug> std::error::Error for SendError<T> {}

        // ── UnboundedSender ──

        /// Wrapper around futures_channel::mpsc::UnboundedSender
        /// that provides a tokio-compatible API (synchronous send).
        #[derive(Debug)]
        pub struct UnboundedSender<T> {
            inner: futures_channel::mpsc::UnboundedSender<T>,
        }

        impl<T> Clone for UnboundedSender<T> {
            fn clone(&self) -> Self {
                Self { inner: self.inner.clone() }
            }
        }

        impl<T> UnboundedSender<T> {
            pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
                self.inner
                    .unbounded_send(msg)
                    .map_err(|e| SendError(e.into_inner()))
            }
        }

        // ── UnboundedReceiver ──

        pub struct UnboundedReceiver<T> {
            inner: futures_channel::mpsc::UnboundedReceiver<T>,
        }

        impl<T> std::fmt::Debug for UnboundedReceiver<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("UnboundedReceiver").finish()
            }
        }

        impl<T> UnboundedReceiver<T> {
            pub async fn recv(&mut self) -> Option<T> {
                use std::future::Future;
                use futures_core::Stream;
                struct RecvFuture<'a, T> {
                    rx: &'a mut futures_channel::mpsc::UnboundedReceiver<T>,
                }
                impl<T> Future for RecvFuture<'_, T> {
                    type Output = Option<T>;
                    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
                        let this = self.get_mut();
                        Pin::new(&mut *this.rx).poll_next(cx)
                    }
                }
                RecvFuture { rx: &mut self.inner }.await
            }

            pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
                match self.inner.try_next() {
                    Ok(Some(val)) => Ok(val),
                    Ok(None) => Err(TryRecvError::Disconnected),
                    Err(_) => Err(TryRecvError::Empty),
                }
            }
        }

        impl<T> futures_core::stream::Stream for UnboundedReceiver<T> {
            type Item = T;
            fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
                Pin::new(&mut self.get_mut().inner).poll_next(cx)
            }
        }

        #[derive(Debug)]
        pub enum TryRecvError {
            Empty,
            Disconnected,
        }

        pub fn unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
            let (tx, rx) = futures_channel::mpsc::unbounded();
            (
                UnboundedSender { inner: tx },
                UnboundedReceiver { inner: rx },
            )
        }

        // ── Sender (bounded-compatible, async send) ──

        /// Bounded-channel compatible sender.
        /// On WASM, backed by an unbounded channel but provides an async `send()`
        /// to match tokio's bounded `Sender::send()` API.
        #[derive(Debug)]
        pub struct Sender<T> {
            inner: futures_channel::mpsc::UnboundedSender<T>,
        }

        impl<T> Clone for Sender<T> {
            fn clone(&self) -> Self {
                Self { inner: self.inner.clone() }
            }
        }

        impl<T> Sender<T> {
            pub async fn send(&self, msg: T) -> Result<(), SendError<T>> {
                self.inner
                    .unbounded_send(msg)
                    .map_err(|e| SendError(e.into_inner()))
            }

            pub fn try_send(&self, msg: T) -> Result<(), SendError<T>> {
                self.inner
                    .unbounded_send(msg)
                    .map_err(|e| SendError(e.into_inner()))
            }
        }

        /// Bounded-channel compatible receiver.
        pub struct Receiver<T> {
            inner: futures_channel::mpsc::UnboundedReceiver<T>,
        }

        impl<T> std::fmt::Debug for Receiver<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("Receiver").finish()
            }
        }

        impl<T> Receiver<T> {
            pub async fn recv(&mut self) -> Option<T> {
                use std::future::Future;
                use futures_core::Stream;
                struct RecvFuture<'a, T> {
                    rx: &'a mut futures_channel::mpsc::UnboundedReceiver<T>,
                }
                impl<T> Future for RecvFuture<'_, T> {
                    type Output = Option<T>;
                    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
                        let this = self.get_mut();
                        Pin::new(&mut *this.rx).poll_next(cx)
                    }
                }
                RecvFuture { rx: &mut self.inner }.await
            }

            pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
                match self.inner.try_next() {
                    Ok(Some(val)) => Ok(val),
                    Ok(None) => Err(TryRecvError::Disconnected),
                    Err(_) => Err(TryRecvError::Empty),
                }
            }
        }

        impl<T> futures_core::stream::Stream for Receiver<T> {
            type Item = T;
            fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
                Pin::new(&mut self.get_mut().inner).poll_next(cx)
            }
        }

        /// Create a bounded channel (on WASM this is actually unbounded).
        pub fn channel<T>(_buffer: usize) -> (Sender<T>, Receiver<T>) {
            let (tx, rx) = futures_channel::mpsc::unbounded();
            (Sender { inner: tx }, Receiver { inner: rx })
        }
    }
    pub mod oneshot {
        pub use futures_channel::oneshot::{Receiver, Sender, channel};
    }
}

// ── Spawn ──

#[cfg(feature = "native")]
pub use tokio::task::JoinHandle;

#[cfg(feature = "native")]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub struct JoinHandle<T> {
    _marker: std::marker::PhantomData<T>,
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl<T> std::fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinHandle").finish()
    }
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl<T> JoinHandle<T> {
    pub fn abort(&self) {}
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl<T> std::future::Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // WASM JoinHandle is a fire-and-forget handle; awaiting it always yields Pending
        // (the spawned task runs to completion independently).
        std::task::Poll::Pending
    }
}

/// Error returned when a JoinHandle fails (WASM stub).
#[cfg(all(feature = "wasm", not(feature = "native")))]
#[derive(Debug)]
pub struct JoinError;

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JoinError")
    }
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl std::error::Error for JoinError {}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        future.await;
    });
    JoinHandle {
        _marker: std::marker::PhantomData,
    }
}

// ── CancellationToken ──

#[cfg(feature = "native")]
pub use tokio_util::sync::CancellationToken;

#[cfg(all(feature = "wasm", not(feature = "native")))]
mod cancellation {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Waker;
    use parking_lot::Mutex;

    /// Lightweight cancellation token for WASM with waker support.
    #[derive(Clone)]
    pub struct CancellationToken {
        cancelled: Arc<AtomicBool>,
        wakers: Arc<Mutex<Vec<Waker>>>,
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
            }
        }

        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            // Wake all registered wakers
            let wakers = std::mem::take(&mut *self.wakers.lock());
            for waker in wakers {
                waker.wake();
            }
        }

        pub fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        pub fn child_token(&self) -> Self {
            self.clone()
        }

        /// Register a waker to be notified when the token is cancelled.
        pub fn register_waker(&self, waker: &Waker) {
            if self.is_cancelled() {
                waker.wake_by_ref();
                return;
            }
            let mut wakers = self.wakers.lock();
            // Deduplicate by checking if the waker is already registered
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
                        // Double-check after registration to avoid race
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

// ── Status type ──

pub use slim_datapath::Status;

// ── Time ──

pub use std::time::Duration;

#[cfg(feature = "native")]
pub async fn sleep(duration: std::time::Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub async fn sleep(duration: std::time::Duration) {
    let millis = duration.as_millis().min(u32::MAX as u128) as u32;
    gloo_timers::future::TimeoutFuture::new(millis).await;
}

/// Result of a select between a timer and a cancellation token.
pub enum SelectResult {
    TimerExpired,
    Cancelled,
}

/// Race a sleep against a cancellation token.
pub async fn select_timer_or_cancel(
    duration: std::time::Duration,
    cancellation_token: &CancellationToken,
) -> SelectResult {
    #[cfg(feature = "native")]
    {
        tokio::select! {
            _ = tokio::time::sleep(duration) => SelectResult::TimerExpired,
            _ = cancellation_token.cancelled() => SelectResult::Cancelled,
        }
    }
    #[cfg(all(feature = "wasm", not(feature = "native")))]
    {
        use std::task::{Context, Poll};

        let millis = duration.as_millis().min(u32::MAX as u128) as u32;
        let timer = gloo_timers::future::TimeoutFuture::new(millis);
        let cancel = cancellation_token.cancelled();

        // Pin both futures on the stack
        let mut timer = core::pin::pin!(timer);
        let mut cancel = core::pin::pin!(cancel);

        // Poll both futures, returning whichever completes first
        std::future::poll_fn(|cx: &mut Context<'_>| {
            if let Poll::Ready(()) = timer.as_mut().poll(cx) {
                return Poll::Ready(SelectResult::TimerExpired);
            }
            if let Poll::Ready(()) = cancel.as_mut().poll(cx) {
                return Poll::Ready(SelectResult::Cancelled);
            }
            Poll::Pending
        })
        .await
    }
}

/// Race a receiver against a cancellation token.
/// Returns `Some(msg)` if a message arrived, `None` if cancelled or channel closed.
pub async fn select_recv_or_cancel<T>(
    rx: &mut channel::mpsc::Receiver<T>,
    cancellation_token: &CancellationToken,
) -> Option<T> {
    #[cfg(feature = "native")]
    {
        tokio::select! {
            msg = rx.recv() => msg,
            _ = cancellation_token.cancelled() => None,
        }
    }
    #[cfg(all(feature = "wasm", not(feature = "native")))]
    {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use futures_core::Stream;

        let cancel = cancellation_token.cancelled();
        let mut cancel = core::pin::pin!(cancel);

        std::future::poll_fn(|cx: &mut Context<'_>| {
            // Check for messages first
            if let Poll::Ready(msg) = Pin::new(&mut *rx).poll_next(cx) {
                return Poll::Ready(msg);
            }
            if let Poll::Ready(()) = cancel.as_mut().poll(cx) {
                return Poll::Ready(None);
            }
            Poll::Pending
        })
        .await
    }
}

// ── Deadline (resettable timer) ──

/// A resettable deadline timer.
///
/// On native, wraps `tokio::time::Sleep`.
/// On WASM, tracks a deadline instant and polls with gloo-timers.
#[cfg(feature = "native")]
pub struct Deadline {
    inner: std::pin::Pin<Box<tokio::time::Sleep>>,
}

#[cfg(feature = "native")]
impl Deadline {
    /// Create a deadline that expires after `duration`.
    pub fn after(duration: std::time::Duration) -> Self {
        Self {
            inner: Box::pin(tokio::time::sleep(duration)),
        }
    }

    /// Reset the deadline to expire `duration` from now.
    pub fn reset(&mut self, duration: std::time::Duration) {
        self.inner
            .as_mut()
            .reset(tokio::time::Instant::now() + duration);
    }

    /// Poll the deadline. Returns `Poll::Ready(())` when expired.
    pub fn poll_expired(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        self.inner.as_mut().poll(cx)
    }
}

#[cfg(feature = "native")]
impl std::future::Future for Deadline {
    type Output = ();
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        self.inner.as_mut().poll(cx)
    }
}

// Make `&mut Deadline` work in tokio::select! via `Unpin`
#[cfg(feature = "native")]
impl Unpin for Deadline {}

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub struct Deadline {
    /// None means the deadline is effectively infinite (not set).
    remaining_ms: Option<u32>,
    /// Pending timer future, replaced on each reset.
    timer: Option<core::pin::Pin<Box<gloo_timers::future::TimeoutFuture>>>,
}

#[cfg(all(feature = "wasm", not(feature = "native")))]
impl Deadline {
    pub fn after(duration: std::time::Duration) -> Self {
        let ms = duration.as_millis().min(u32::MAX as u128) as u32;
        // For very large durations (> 1 day), treat as "infinite"
        if ms >= 86_400_000 {
            Self {
                remaining_ms: None,
                timer: None,
            }
        } else {
            Self {
                remaining_ms: Some(ms),
                timer: Some(Box::pin(gloo_timers::future::TimeoutFuture::new(ms))),
            }
        }
    }

    pub fn reset(&mut self, duration: std::time::Duration) {
        let ms = duration.as_millis().min(u32::MAX as u128) as u32;
        self.remaining_ms = Some(ms);
        self.timer = Some(Box::pin(gloo_timers::future::TimeoutFuture::new(ms)));
    }

    pub fn poll_expired(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        use std::task::Poll;
        match self.timer.as_mut() {
            Some(timer) => timer.as_mut().poll(cx),
            None => Poll::Pending, // infinite deadline never fires
        }
    }
}

/// Result of the 3-way processing loop select.
pub enum ProcessingSelect<T> {
    /// Cancellation token was triggered.
    Cancelled,
    /// The deadline expired.
    DeadlineExpired,
    /// A message was received (None if channel closed).
    Message(Option<T>),
}

/// Three-way select for the session processing loop pattern:
/// cancel vs deadline vs recv.
pub async fn select_processing<T>(
    rx: &mut channel::mpsc::Receiver<T>,
    cancellation_token: &CancellationToken,
    deadline: &mut Deadline,
    check_cancel: bool,
) -> ProcessingSelect<T> {
    #[cfg(feature = "native")]
    {
        if check_cancel {
            tokio::select! {
                _ = cancellation_token.cancelled() => ProcessingSelect::Cancelled,
                _ = &mut *deadline => ProcessingSelect::DeadlineExpired,
                msg = rx.recv() => ProcessingSelect::Message(msg),
            }
        } else {
            tokio::select! {
                _ = &mut *deadline => ProcessingSelect::DeadlineExpired,
                msg = rx.recv() => ProcessingSelect::Message(msg),
            }
        }
    }
    #[cfg(all(feature = "wasm", not(feature = "native")))]
    {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use futures_core::Stream;

        std::future::poll_fn(|cx: &mut Context<'_>| {
            // Check cancellation first (if active)
            if check_cancel {
                if cancellation_token.is_cancelled() {
                    return Poll::Ready(ProcessingSelect::Cancelled);
                }
                // Register waker so we get notified on cancel
                cancellation_token.register_waker(cx.waker());
            }

            // Check deadline
            if let Poll::Ready(()) = deadline.poll_expired(cx) {
                return Poll::Ready(ProcessingSelect::DeadlineExpired);
            }

            // Check for messages
            if let Poll::Ready(msg) = Pin::new(&mut *rx).poll_next(cx) {
                return Poll::Ready(ProcessingSelect::Message(msg));
            }

            Poll::Pending
        })
        .await
    }
}
