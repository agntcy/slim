// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use thiserror::Error;

use tokio::sync::oneshot;

use slim_datapath::api::{ProtoMessage as Message, ProtoSubscriptionAck};
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;

use crate::common::SlimChannelSender;

/// How long to wait for a subscription ACK before giving up.
///
/// The datapath retry loop runs `0..=MAX_RETRIES` attempts (currently 4) with a
/// per-attempt timeout of `TIMEOUT` (currently 2 s), for a maximum of
/// `TIMEOUT * (MAX_RETRIES + 1) = 8 s`.  This deadline must be at least that
/// large so every retry attempt has a chance to succeed before the session
/// considers the operation lost.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Error, Debug)]
pub enum SubscriptionAckError {
    #[error("ack rejected by datapath: {message}")]
    Rejected { message: String },
    #[error("ack channel closed")]
    ChannelClosed,
    #[error("ack timed out")]
    Timeout,
}

/// Trait that abstracts subscription and route management operations.
///
/// Every method sends the request with an ack_id and returns the
/// [`oneshot::Receiver`] for that ACK.  The caller decides whether to await
/// the receiver immediately (blocking until confirmed) or drop it (fire and
/// forget while the datapath still tracks the operation).
#[async_trait]
pub trait SubscriptionOps: Clone + Send + Sync + 'static {
    /// Subscribe (forward_to): register interest in `name`, optionally routing
    /// through a specific connection.
    async fn subscribe(
        &self,
        source: &Name,
        name: &Name,
        forward_to: Option<u64>,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>;

    /// Unsubscribe (forward_to): de-register interest in `name`.
    async fn unsubscribe(
        &self,
        source: &Name,
        name: &Name,
        subscription_id: u64,
        forward_to: Option<u64>,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError>;

    /// Set a recv_from route for `name` on connection `conn`.
    async fn set_route(
        &self,
        source: &Name,
        name: &Name,
        conn: u64,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>;

    /// Remove a recv_from route for `name` on connection `conn`.
    async fn remove_route(
        &self,
        source: &Name,
        name: &Name,
        subscription_id: u64,
        conn: u64,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError>;

    /// Called during session stack construction to create a default instance
    /// from the SLIM channel sender.  Returns `None` if this type requires
    /// explicit construction (caller must call `with_subscription_manager` on
    /// the builder).
    fn from_slim_tx(_tx: &SlimChannelSender) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

/// A no-op subscription manager for tests that do not run a real SLIM
/// datapath.  Every operation immediately succeeds without sending any
/// messages.
#[derive(Clone)]
pub struct AutoAckManager {
    ack_counter: Arc<AtomicU64>,
}

#[async_trait]
impl SubscriptionOps for AutoAckManager {
    async fn subscribe(
        &self,
        _source: &Name,
        _name: &Name,
        _forward_to: Option<u64>,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let id = self.ack_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok((id, rx))
    }

    async fn unsubscribe(
        &self,
        _source: &Name,
        _name: &Name,
        _subscription_id: u64,
        _forward_to: Option<u64>,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok(rx)
    }

    async fn set_route(
        &self,
        _source: &Name,
        _name: &Name,
        _conn: u64,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let id = self.ack_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok((id, rx))
    }

    async fn remove_route(
        &self,
        _source: &Name,
        _name: &Name,
        _subscription_id: u64,
        _conn: u64,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok(rx)
    }

    fn from_slim_tx(_tx: &SlimChannelSender) -> Option<Self> {
        Some(AutoAckManager {
            ack_counter: Arc::new(AtomicU64::new(0)),
        })
    }
}

#[derive(Clone)]
pub struct SubscriptionManager {
    pub pending_acks: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<(), SubscriptionAckError>>>>>,
    ack_counter: Arc<AtomicU64>,
    tx: SlimChannelSender,
}

#[async_trait]
impl SubscriptionOps for SubscriptionManager {
    async fn subscribe(
        &self,
        source: &Name,
        name: &Name,
        forward_to: Option<u64>,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let source = source.clone();
        let name = name.clone();
        self.send_with_receiver(move |ack_id| {
            let flags = if let Some(conn) = forward_to {
                SlimHeaderFlags::default().with_forward_to(conn)
            } else {
                SlimHeaderFlags::default()
            };
            Message::builder()
                .source(source)
                .destination(name)
                .flags(flags)
                .subscription_id(ack_id)
                .build_subscribe()
                .unwrap()
        })
        .await
    }

    async fn unsubscribe(
        &self,
        source: &Name,
        name: &Name,
        subscription_id: u64,
        forward_to: Option<u64>,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let source = source.clone();
        let name = name.clone();
        self.send_with_id(subscription_id, move |ack_id| {
            let flags = if let Some(conn) = forward_to {
                SlimHeaderFlags::default().with_forward_to(conn)
            } else {
                SlimHeaderFlags::default()
            };
            Message::builder()
                .source(source)
                .destination(name)
                .flags(flags)
                .subscription_id(ack_id)
                .build_unsubscribe()
                .unwrap()
        })
        .await
    }

    async fn set_route(
        &self,
        source: &Name,
        name: &Name,
        conn: u64,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let source = source.clone();
        let name = name.clone();
        self.send_with_receiver(move |ack_id| {
            Message::builder()
                .source(source)
                .destination(name)
                .flags(SlimHeaderFlags::default().with_recv_from(conn))
                .subscription_id(ack_id)
                .build_subscribe()
                .unwrap()
        })
        .await
    }

    async fn remove_route(
        &self,
        source: &Name,
        name: &Name,
        subscription_id: u64,
        conn: u64,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let source = source.clone();
        let name = name.clone();
        self.send_with_id(subscription_id, move |ack_id| {
            Message::builder()
                .source(source)
                .destination(name)
                .flags(SlimHeaderFlags::default().with_recv_from(conn))
                .subscription_id(ack_id)
                .build_unsubscribe()
                .unwrap()
        })
        .await
    }

    fn from_slim_tx(tx: &SlimChannelSender) -> Option<Self> {
        Some(SubscriptionManager::new(tx.clone()))
    }
}

/// Spy subscription manager for tests: immediately returns `Ok(())` and
/// records each call to a channel so tests can assert on the operations.
#[cfg(test)]
#[derive(Clone)]
pub struct SpySubscriptionManager {
    tx: Arc<tokio::sync::mpsc::UnboundedSender<SubscriptionCall>>,
}

/// Individual subscription operation recorded by [`SpySubscriptionManager`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionCall {
    Subscribe,
    Unsubscribe,
    SetRoute,
    RemoveRoute,
}

#[cfg(test)]
impl SpySubscriptionManager {
    pub fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<SubscriptionCall>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { tx: Arc::new(tx) }, rx)
    }
}

#[cfg(test)]
#[async_trait]
impl SubscriptionOps for SpySubscriptionManager {
    async fn subscribe(
        &self,
        _source: &Name,
        _name: &Name,
        _forward_to: Option<u64>,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let _ = self.tx.send(SubscriptionCall::Subscribe);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok((0, rx))
    }

    async fn unsubscribe(
        &self,
        _source: &Name,
        _name: &Name,
        _subscription_id: u64,
        _forward_to: Option<u64>,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let _ = self.tx.send(SubscriptionCall::Unsubscribe);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok(rx)
    }

    async fn set_route(
        &self,
        _source: &Name,
        _name: &Name,
        _conn: u64,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let _ = self.tx.send(SubscriptionCall::SetRoute);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok((0, rx))
    }

    async fn remove_route(
        &self,
        _source: &Name,
        _name: &Name,
        _subscription_id: u64,
        _conn: u64,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let _ = self.tx.send(SubscriptionCall::RemoveRoute);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        Ok(rx)
    }

    fn from_slim_tx(_tx: &SlimChannelSender) -> Option<Self> {
        None
    }
}

impl SubscriptionManager {
    pub fn new(tx: SlimChannelSender) -> Self {
        Self {
            pending_acks: Arc::new(Mutex::new(HashMap::new())),
            ack_counter: Arc::new(AtomicU64::new(rand::random::<u64>())),
            tx,
        }
    }

    fn next_ack_id(&self) -> u64 {
        self.ack_counter.fetch_add(1, Ordering::Relaxed) + 1
    }

    async fn send_with_receiver(
        &self,
        build_message: impl FnOnce(u64) -> Message,
    ) -> Result<(u64, oneshot::Receiver<Result<(), SubscriptionAckError>>), SubscriptionAckError>
    {
        let ack_id = self.next_ack_id();
        let (ack_tx, ack_rx) = oneshot::channel();
        {
            let mut pending = self.pending_acks.lock();
            pending.insert(ack_id, ack_tx);
        }

        let msg = build_message(ack_id);

        if self.tx.send(Ok(msg)).await.is_err() {
            self.pending_acks.lock().remove(&ack_id);
            return Err(SubscriptionAckError::ChannelClosed);
        }

        Ok((ack_id, ack_rx))
    }

    async fn send_with_id(
        &self,
        subscription_id: u64,
        build_message: impl FnOnce(u64) -> Message,
    ) -> Result<oneshot::Receiver<Result<(), SubscriptionAckError>>, SubscriptionAckError> {
        let ack_rx = self.register_ack_with_id(subscription_id);

        let msg = build_message(subscription_id);

        if self.tx.send(Ok(msg)).await.is_err() {
            self.pending_acks.lock().remove(&subscription_id);
            return Err(SubscriptionAckError::ChannelClosed);
        }

        Ok(ack_rx)
    }

    /// Register a pending ACK entry and return the ack_id and receiver.
    /// The caller is responsible for building and sending the message with this ack_id.
    /// If sending fails, call `cancel_ack` to clean up.
    pub fn register_ack(&self) -> (u64, oneshot::Receiver<Result<(), SubscriptionAckError>>) {
        let ack_id = self.next_ack_id();
        let (ack_tx, ack_rx) = oneshot::channel();
        {
            let mut pending = self.pending_acks.lock();
            pending.insert(ack_id, ack_tx);
        }
        (ack_id, ack_rx)
    }

    /// Register a pending ACK entry under a caller-provided ID and return the receiver.
    pub fn register_ack_with_id(
        &self,
        id: u64,
    ) -> oneshot::Receiver<Result<(), SubscriptionAckError>> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_acks.lock().insert(id, ack_tx);
        ack_rx
    }

    /// Remove a previously registered pending ACK (call on send failure).
    pub fn cancel_ack(&self, ack_id: u64) {
        let mut pending = self.pending_acks.lock();
        pending.remove(&ack_id);
    }

    /// Await a previously registered ACK receiver, with a deadline of [`ACK_TIMEOUT`].
    pub async fn await_ack(
        ack_rx: oneshot::Receiver<Result<(), SubscriptionAckError>>,
    ) -> Result<(), SubscriptionAckError> {
        match tokio::time::timeout(ACK_TIMEOUT, ack_rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(SubscriptionAckError::ChannelClosed),
            Err(_) => Err(SubscriptionAckError::Timeout),
        }
    }

    /// Called by the App message loop to complete a waiting future for an ACK.
    pub fn resolve_ack(&self, ack: &ProtoSubscriptionAck) {
        tracing::debug!(ack = %ack.subscription_id, "ack received");
        let sender = {
            let mut pending = self.pending_acks.lock();
            pending.remove(&ack.subscription_id)
        };

        if let Some(sender) = sender {
            let _ = sender.send(if ack.success {
                Ok(())
            } else {
                Err(SubscriptionAckError::Rejected {
                    message: if ack.error.is_empty() {
                        "subscription ack failed".to_string()
                    } else {
                        ack.error.clone()
                    },
                })
            });
        } else {
            tracing::info!(
                ack_id = %ack.subscription_id,
                "received subscription ack with no pending waiter"
            );
        }
    }
}
