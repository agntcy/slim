// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::marker::PhantomData;

use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_datapath::{
    api::{ProtoMessage as Message, SessionHeader, SlimHeader},
    messages::Name,
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
    MessageDirection, SessionError, SessionType, Transmitter,
    common::SessionMessage,
    receiver_buffer::ReceiverBuffer,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
};

#[allow(dead_code)]
struct ReliableReceiverInternal<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// buffer with received packets one per endpoint
    buffer: HashMap<Name, ReceiverBuffer>,

    /// list of pending RTX requests per name/id
    pending_rtxs: HashMap<Name, HashMap<u32, (Timer, Message)>>,

    /// timer factory to crate timers for rtx
    /// if None, no rtx is sent. In this case there is no
    /// ordered delivery to the app and messages are sent
    /// as soon as they arrive at there received without using
    /// the buffer
    timer_factory: Option<TimerFactory>,

    /// session id where to send the messages
    session_id: u32,

    /// local name to use as source for the rtx messages
    local_name: Name,

    /// session type
    session_type: SessionType,

    /// if true send acks for each received message
    send_acks: bool,

    /// send to slim/app
    tx: T,

    /// received for signals
    rx: Receiver<SessionMessage>,

    /// drain state - when true, no new messages from app are accepted
    is_draining: bool,

    /// drain timer - when set, we're waiting for pending acks during grace period
    drain_timer: Option<Timer>,
}

#[allow(dead_code)]
impl<T> ReliableReceiverInternal<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// to create the timer factory and send rtx messages
    /// timer_settings and tx_timer must be not null
    #[allow(clippy::too_many_arguments)]
    fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        local_name: Name,
        session_type: SessionType,
        send_acks: bool,
        tx: T,
        tx_signals: Option<Sender<SessionMessage>>,
        rx_signals: Receiver<SessionMessage>,
    ) -> Self {
        let factory = if let Some(settings) = timer_settings
            && let Some(tx) = tx_signals
        {
            Some(TimerFactory::new(settings, tx))
        } else {
            None
        };

        ReliableReceiverInternal {
            buffer: HashMap::new(),
            pending_rtxs: HashMap::new(),
            timer_factory: factory,
            session_id,
            session_type,
            local_name,
            send_acks,
            tx,
            rx: rx_signals,
            is_draining: false,
            drain_timer: None,
        }
    }

    /// TODO: should we always pass on this objects even if we are not reliable??
    /// I think it is a good idea but in that case we should call them just sender and receiver
    async fn on_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            //slim_datapath::api::ProtoSessionMessageType::P2PMsg => {
            //    debug!("received P2P reliable message");
            //    self.on_publish_message(message).await?
            //}
            slim_datapath::api::ProtoSessionMessageType::P2PReliable => {
                debug!("received P2P reliable message");
                self.on_publish_message(message).await?
            }
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg => {
                debug!("received multicast message");
                self.on_publish_message(message).await?
            }
            slim_datapath::api::ProtoSessionMessageType::RtxReply => {
                debug!("received rtx message");
                self.on_rtx_message(message).await?
            }
            _ => {
                // TODO: here we need to handle also the Channel messages (Acks + Replies)
                // so that we can use this code for timers
                debug!("unexpected message type");
            }
        }
        Ok(())
    }

    async fn on_publish_message(&mut self, message: Message) -> Result<(), SessionError> {
        if self.timer_factory.is_none() {
            debug!(
                "received message {} from {}, send it to the app without reordering",
                message.get_id(),
                message.get_source()
            );
            return self.tx.send_to_app(Ok(message)).await;
        }

        let source = message.get_source();
        let in_conn = message.get_incoming_conn();
        let buffer = match self.buffer.get_mut(&source) {
            Some(b) => b,
            None => {
                self.buffer
                    .insert(source.clone(), ReceiverBuffer::default());
                self.buffer.get_mut(&source).unwrap()
            }
        };

        let (recv_vec, rtx_vec) = buffer.on_received_message(message);
        self.handle_recv_and_rtx_vectors(&source, in_conn, recv_vec, rtx_vec)
            .await
    }

    async fn on_rtx_message(&mut self, message: Message) -> Result<(), SessionError> {
        // in case we get the and RTX reply the session must be relianle
        let source = message.get_source();
        let id = message.get_id();
        let in_conn = message.get_incoming_conn();

        debug!("received RTX reply for message {} from {}", id, source);

        // remote the timer
        if let Some(pending) = self.pending_rtxs.get_mut(&source)
            && let Some((t, _m)) = pending.get_mut(&id)
        {
            t.stop();
            pending.remove(&id);
        }

        // if rtx is not an error pass to on_publish_message
        // otherwise mange the message loss
        if message.get_error().is_none() {
            return self.on_publish_message(message).await;
        }

        let buffer = self.buffer.get_mut(&source).ok_or_else(|| {
            SessionError::Processing("missing receiver buffer for incoming rtx reply".to_string())
        })?;
        let recv_vec = buffer.on_lost_message(id);
        self.handle_recv_and_rtx_vectors(&source, in_conn, recv_vec, vec![])
            .await
    }

    async fn handle_recv_and_rtx_vectors(
        &mut self,
        source: &Name,
        in_conn: u64,
        recv_vec: Vec<Option<Message>>,
        rtx_vec: Vec<u32>,
    ) -> Result<(), SessionError> {
        for recv in recv_vec {
            match recv {
                Some(r) => {
                    debug!(
                        "received message {} from {}, send it to the app",
                        r.get_id(),
                        r.get_source()
                    );
                    self.tx.send_to_app(Ok(r)).await?;
                }
                None => {
                    debug!(
                        "lost message from {} on session {}",
                        source, self.session_id
                    );
                    self.tx
                        .send_to_app(Err(SessionError::MessageLost(self.session_id.to_string())))
                        .await?;
                }
            }
        }

        for rtx_id in rtx_vec {
            debug!("send rtx for message id {} to {}", rtx_id, source);

            // create RTX packet
            let slim_header = Some(SlimHeader::new(
                &self.local_name,
                source,
                Some(SlimHeaderFlags::default().with_forward_to(in_conn)),
            ));

            let session_header = Some(SessionHeader::new(
                self.session_type.clone() as i32,
                slim_datapath::api::ProtoSessionMessageType::RtxRequest.into(),
                self.session_id,
                rtx_id,
                &None,
                &None,
            ));

            let rtx = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

            // for each RTX start a timer
            debug!("create rtx timer for message {} form {}", rtx_id, source);
            let pending = match self.pending_rtxs.get_mut(source) {
                Some(p) => p,
                None => {
                    self.pending_rtxs.insert(source.clone(), HashMap::new());
                    self.pending_rtxs.get_mut(source).unwrap()
                }
            };

            let timer = self
                .timer_factory
                .as_ref()
                .unwrap()
                .create_and_start_timer(rtx_id, Some(source.clone()));
            pending.insert(rtx_id, (timer, rtx.clone()));

            // send message
            debug!("send rtx request for message {} to {}", rtx_id, source);
            self.tx.send_to_slim(Ok(rtx)).await?;
        }

        Ok(())
    }

    async fn on_timer_timeout(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!("timeout for message {} from {}", id, name);
        let pending = self.pending_rtxs.get_mut(&name).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        let (_t, rtx) = pending.get(&id).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        debug!("send rtx request again");
        self.tx.send_to_slim(Ok(rtx.clone())).await
    }

    async fn on_timer_failure(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!(
            "timer failure for message {} from {}, clear state",
            id, name
        );
        let pending = self.pending_rtxs.get_mut(&name).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        let (t, _rtx) = pending.get_mut(&id).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        // stop the timer and remove the name if no pending rtx left
        t.stop();
        pending.remove(&id);
        if pending.is_empty() {
            self.pending_rtxs.remove(&name);
        }

        // notify the application that the message was not delivered correctly
        self.tx
            .send_to_app(Err(SessionError::Processing(format!(
                "error send message {}. stop retrying",
                id
            ))))
            .await
    }

    fn start_drain(&mut self, grace_period_ms: u64) {
        debug!("starting drain with grace period {}ms", grace_period_ms);
        self.is_draining = true;

        // if timer_factory is None, we can close the receiver
        if self.timer_factory.is_none() {
            debug!("no pending acks, can stop immediately");
            // No pending acks, we can stop immediately
            self.drain_timer = None;
        }

        // If we have pending rtx, start a grace period timer
        if !self.pending_rtxs.is_empty() {
            debug!("pending rtx exist, starting grace period timer");
            // Use a special timer ID for drain (u32::MAX to avoid conflicts)
            let drain_timer_id = u32::MAX;
            let timer = Timer::new(
                drain_timer_id,
                crate::timer::TimerType::Constant,
                std::time::Duration::from_millis(grace_period_ms),
                None,
                Some(1), // max_retries - only fire once for drain
            );
            self.timer_factory
                .as_ref()
                .unwrap()
                .start_timer(&timer, None);
            self.drain_timer = Some(timer);
        } else {
            debug!("no pending acks, can stop immediately");
            // No pending acks, we can stop immediately
            self.drain_timer = None;
        }
    }

    fn check_drain_completion(&self) -> bool {
        // Drain is complete if we're draining and no pending rtx remain
        self.is_draining && self.pending_rtxs.is_empty()
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                message = self.rx.recv() => {
                    match message {
                        Some(SessionMessage::OnMessage { message, direction: _ }) => {
                            if self.on_message(message).await.is_err() {
                                error!("Error receiving message");
                            }
                            // Check if drain is complete after processing messages (especially rtx)
                            if self.check_drain_completion() {
                                debug!("drain completed, all rtx received");
                                break;
                            }
                        }
                        Some(SessionMessage::TimerTimeout { message_id, timeouts: _, name }) => {
                            // Check if this is the drain timer
                            if message_id == u32::MAX {
                                debug!("drain grace period expired, stopping sender");
                                break; // Exit the loop to stop the sender
                            }
                            if let Some(name) = name {
                                if let Err(e) = self.on_timer_timeout(message_id, name).await {
                                    debug!("Error handling timer timeout: {:?}", e);
                                }
                            } else {
                                error!("receive timeout without a name, do not process it");
                            }
                        }
                        Some(SessionMessage::TimerFailure { message_id, timeouts: _, name }) => {
                            if let Some(name) = name {
                                if let Err(e) = self.on_timer_failure(message_id, name).await {
                                    debug!("Error handling timer failure: {:?}", e);
                                }
                            } else {
                                error!("receive timer failure without a name, do not process it");
                            }
                        }
                        Some(SessionMessage::Drain { grace_period_ms }) => {
                            debug!("Drain message received for ReliableReceiver");
                            self.start_drain(grace_period_ms);
                            if self.drain_timer.is_none() {
                                // close the sender
                                break;
                            }
                        }
                        Some(_) => {
                            debug!("unexpected message type");
                        }
                        None => {
                            debug!("Channel closed, stopping ReliableReceiver");
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) struct ReliableReceiver<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    tx: Sender<SessionMessage>,
    _phantom: PhantomData<T>,
}

#[allow(dead_code)]
impl<T> ReliableReceiver<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        local_name: Name,
        session_type: SessionType,
        send_acks: bool,
        tx_transmitter: T,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        let reliable_receiver = ReliableReceiverInternal::new(
            timer_settings,
            session_id,
            local_name,
            session_type,
            send_acks,
            tx_transmitter,
            Some(tx.clone()),
            rx,
        );

        // Spawn the processing loop
        tokio::spawn(async move {
            reliable_receiver.process_loop().await;
        });

        ReliableReceiver {
            tx,
            _phantom: PhantomData,
        }
    }

    /// To be used to handle messages from slim
    pub async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx
            .send(SessionMessage::OnMessage { message, direction })
            .await
            .map_err(|_| SessionError::Processing("Failed to queue message".to_string()))
    }
}
