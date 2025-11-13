// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::sync::oneshot;

use crate::SessionError;

/// The inner future type that can be awaited for completion.
#[derive(Debug)]
enum CompletionFuture {
    /// A oneshot receiver for completion acknowledgments
    OneshotReceiver(oneshot::Receiver<Result<(), SessionError>>),
    /// A join handle for task-based completions
    JoinHandle(tokio::task::JoinHandle<()>),
}

/// A handle to await the completion of an asynchronous operation.
///
/// This type wraps an internal future and can be directly awaited
/// to check if an operation completed successfully. It is used for:
/// - Message delivery acknowledgments
/// - Session initialization (e.g., P2P handshakes)
/// - Participant invitations
/// - Participant removals
///
/// # Examples
///
/// Basic usage with oneshot receiver:
/// ```ignore
/// let ack = session.publish(...).await?;
/// ack.await?; // Wait for delivery confirmation
/// ```
///
/// Usage with join handle:
/// ```ignore
/// let handle = tokio::spawn(async {
///     // Some async work
///     Ok(())
/// });
/// let completion = CompletionHandle::from_join_handle(handle);
/// completion.await?;
/// ```
#[derive(Debug)]
pub struct CompletionHandle {
    inner: CompletionFuture,
}

impl CompletionHandle {
    /// Create a new completion handle from a oneshot receiver.
    ///
    /// This is the most common constructor, used for operations that use
    /// a oneshot channel to signal completion.
    pub fn from_oneshot_receiver(receiver: oneshot::Receiver<Result<(), SessionError>>) -> Self {
        Self {
            inner: CompletionFuture::OneshotReceiver(receiver),
        }
    }

    /// Create a new completion handle from a join handle.
    ///
    /// This is used for operations that spawn a task and need to await
    /// its completion.
    pub fn from_join_handle(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            inner: CompletionFuture::JoinHandle(handle),
        }
    }
}

impl Future for CompletionHandle {
    type Output = Result<(), SessionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match &mut this.inner {
            CompletionFuture::OneshotReceiver(receiver) => match Pin::new(receiver).poll(cx) {
                Poll::Ready(Ok(result)) => Poll::Ready(result),
                Poll::Ready(Err(e)) => Poll::Ready(Err(SessionError::AckReception(e.to_string()))),
                Poll::Pending => Poll::Pending,
            },
            CompletionFuture::JoinHandle(handle) => match Pin::new(handle).poll(cx) {
                Poll::Ready(Ok(result)) => Poll::Ready(Ok(result)),
                Poll::Ready(Err(e)) => Poll::Ready(Err(SessionError::AckReception(format!(
                    "Join handle error: {}",
                    e
                )))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
