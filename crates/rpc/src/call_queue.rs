// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The queue of incoming messages of one call.
//!
//! A session carries many calls, and one task reads all of its messages and
//! hands each to the call it belongs to. That task must never wait on one
//! call's consumer: the others would stall, and so would a cancel. So a call's
//! queue is bounded, and when its consumer falls too far behind, that call
//! fails with `RESOURCE_EXHAUSTED` instead of growing without limit or losing
//! messages.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use tokio::sync::mpsc::{self, error::TrySendError};

use crate::{ReceivedMessage, RpcError};

/// How many messages a call may have queued, unread by its consumer, before the
/// call fails. It applies to the requests a handler has not yet read and to
/// the responses a caller has not yet read.
pub const CALL_QUEUE_LEN: usize = 1024;

/// What became of a message offered to a [`CallQueueTx`].
pub(crate) enum Pushed {
    Queued,
    /// The queue is full. The message was not queued, and the consumer fails
    /// with [`overflow_error`] once it has read what is queued.
    Overflowed,
    /// The consumer is gone.
    Closed,
}

/// The sending end of a call's queue.
pub(crate) struct CallQueueTx {
    tx: mpsc::Sender<ReceivedMessage>,
    overflowed: Arc<AtomicBool>,
}

impl CallQueueTx {
    /// Queue `msg` unless the queue is full. After [`Pushed::Overflowed`] the
    /// caller must drop this sender, which ends the queue for the consumer.
    pub(crate) fn push(&self, msg: ReceivedMessage) -> Pushed {
        match self.tx.try_send(msg) {
            Ok(()) => Pushed::Queued,
            Err(TrySendError::Full(_)) => {
                self.overflowed.store(true, Ordering::Release);
                Pushed::Overflowed
            }
            Err(TrySendError::Closed(_)) => Pushed::Closed,
        }
    }
}

/// The receiving end of a call's queue.
pub struct CallQueueRx {
    rx: mpsc::Receiver<ReceivedMessage>,
    overflowed: Arc<AtomicBool>,
}

impl CallQueueRx {
    pub(crate) async fn recv(&mut self) -> Option<ReceivedMessage> {
        self.rx.recv().await
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<ReceivedMessage>> {
        self.rx.poll_recv(cx)
    }

    /// Whether the queue has ended because it was full rather than because
    /// the call or its session ended. Meaningful once `recv` returns `None`.
    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }
}

/// A queue for one call, holding up to [`CALL_QUEUE_LEN`] messages.
pub(crate) fn call_queue() -> (CallQueueTx, CallQueueRx) {
    call_queue_of(CALL_QUEUE_LEN)
}

fn call_queue_of(len: usize) -> (CallQueueTx, CallQueueRx) {
    let (tx, rx) = mpsc::channel(len);
    let overflowed = Arc::new(AtomicBool::new(false));
    (
        CallQueueTx {
            tx,
            overflowed: overflowed.clone(),
        },
        CallQueueRx { rx, overflowed },
    )
}

/// The error a call fails with when its queue overflowed.
pub(crate) fn overflow_error() -> RpcError {
    RpcError::resource_exhausted(format!(
        "The call fell more than {CALL_QUEUE_LEN} messages behind"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use slim_datapath::api::ProtoName as Name;

    use super::*;

    fn message(n: u8) -> ReceivedMessage {
        ReceivedMessage {
            metadata: HashMap::new(),
            payload: vec![n],
            source: Name::from_strings(["org", "ns", "app"]),
        }
    }

    #[tokio::test]
    async fn test_a_full_queue_overflows_and_keeps_what_it_holds() {
        let (tx, mut rx) = call_queue_of(2);

        assert!(matches!(tx.push(message(1)), Pushed::Queued));
        assert!(matches!(tx.push(message(2)), Pushed::Queued));
        assert!(matches!(tx.push(message(3)), Pushed::Overflowed));
        drop(tx);

        assert_eq!(rx.recv().await.unwrap().payload, vec![1]);
        assert_eq!(rx.recv().await.unwrap().payload, vec![2]);
        assert!(rx.recv().await.is_none());
        assert!(rx.overflowed());
    }

    #[tokio::test]
    async fn test_reading_makes_room() {
        let (tx, mut rx) = call_queue_of(1);

        assert!(matches!(tx.push(message(1)), Pushed::Queued));
        assert_eq!(rx.recv().await.unwrap().payload, vec![1]);
        assert!(matches!(tx.push(message(2)), Pushed::Queued));
        drop(tx);

        assert_eq!(rx.recv().await.unwrap().payload, vec![2]);
        assert!(rx.recv().await.is_none());
        assert!(!rx.overflowed());
    }

    #[tokio::test]
    async fn test_a_queue_whose_consumer_is_gone_is_closed() {
        let (tx, rx) = call_queue_of(1);
        drop(rx);

        assert!(matches!(tx.push(message(1)), Pushed::Closed));
    }
}
