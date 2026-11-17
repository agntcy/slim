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

use crate::{ReceivedMessage, RpcCode, RpcError, STATUS_CODE_KEY};

/// How many messages a call may have queued, unread by its consumer, before the
/// call fails. It applies to the requests a handler has not yet read and to
/// the responses a caller has not yet read. The message that ends the call, an
/// EOS or an error, does not count: a consumer that is only one message behind
/// its end is not a slow one.
pub const CALL_QUEUE_LEN: usize = 1024;

/// Slots kept back for the message that ends the call.
const RESERVED: usize = 1;

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
        // Only this sender takes slots, so the room can only grow between this
        // check and the send.
        if self.tx.capacity() <= RESERVED && !ends_call(&msg) {
            self.overflowed.store(true, Ordering::Release);
            return Pushed::Overflowed;
        }
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
    let (tx, rx) = mpsc::channel(len + RESERVED);
    let overflowed = Arc::new(AtomicBool::new(false));
    (
        CallQueueTx {
            tx,
            overflowed: overflowed.clone(),
        },
        CallQueueRx { rx, overflowed },
    )
}

/// Whether `msg` is the last one its sender sends for the call: an EOS, or an
/// error.
fn ends_call(msg: &ReceivedMessage) -> bool {
    msg.is_eos()
        || RpcCode::from_metadata_str(msg.metadata.get(STATUS_CODE_KEY).map(String::as_str))
            != RpcCode::Ok
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

    fn ending(code: RpcCode, payload: Vec<u8>) -> ReceivedMessage {
        ReceivedMessage {
            metadata: HashMap::from([(STATUS_CODE_KEY.to_string(), (code as i32).to_string())]),
            payload,
            source: Name::from_strings(["org", "ns", "app"]),
        }
    }

    #[tokio::test]
    async fn test_the_message_that_ends_a_call_fits_in_a_full_queue() {
        for end in [
            ending(RpcCode::Ok, vec![]),
            ending(RpcCode::Internal, b"failed".to_vec()),
        ] {
            let (tx, mut rx) = call_queue_of(2);
            assert!(matches!(tx.push(message(1)), Pushed::Queued));
            assert!(matches!(tx.push(message(2)), Pushed::Queued));
            assert!(matches!(tx.push(end), Pushed::Queued));
            drop(tx);

            assert_eq!(rx.recv().await.unwrap().payload, vec![1]);
            assert_eq!(rx.recv().await.unwrap().payload, vec![2]);
            assert!(rx.recv().await.is_some());
            assert!(rx.recv().await.is_none());
            assert!(!rx.overflowed());
        }
    }

    #[tokio::test]
    async fn test_a_data_message_with_an_ok_status_counts_against_the_limit() {
        let (tx, _rx) = call_queue_of(1);
        assert!(matches!(tx.push(message(1)), Pushed::Queued));
        // Unlike an EOS it has a payload, so it is data.
        assert!(matches!(
            tx.push(ending(RpcCode::Ok, vec![2])),
            Pushed::Overflowed
        ));
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
