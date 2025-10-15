// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
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

const DRAIN_TIMER_ID: u32 = 0;

// structs used in the pending rtx map
struct PendingRtxVal {
    timer: Timer,
    message: Message,
}

struct PendingRtxKey {
    name: Name,
    id: u32,
}

impl PartialEq for PendingRtxKey {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.id == other.id
    }
}

impl Eq for PendingRtxKey {}

impl Hash for PendingRtxKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.id.hash(state);
    }
}

#[allow(dead_code)]
struct SessionReceiverInternal<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// buffer with received packets one per endpoint
    buffer: HashMap<Name, ReceiverBuffer>,

    /// list of pending RTX requests per name/id
    pending_rtxs: HashMap<PendingRtxKey, PendingRtxVal>,

    /// timer factory to crate timers for rtx
    /// if None, no rtx is sent. In this case there is no
    /// ordered delivery to the app and messages are sent
    /// as soon as they arrive at there receiver without using
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
impl<T> SessionReceiverInternal<T>
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

        SessionReceiverInternal {
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

    async fn on_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::P2PMsg => {
                debug!("received P2P message");
                if self.timer_factory.is_some() {
                    // this receiver is reliable but got an unrelibale messages
                    // drop the message
                    return Err(SessionError::Processing(
                        "received an unrelibale message on a reliable received".to_string(),
                    ));
                }
                self.on_publish_message(message).await?
            }
            slim_datapath::api::ProtoSessionMessageType::P2PReliable => {
                debug!("received P2P reliable message");
                if self.timer_factory.is_none() {
                    // this receiver is unreliable but got an relibale messages
                    // drop the message
                    return Err(SessionError::Processing(
                        "received a relibale message on an unreliable received".to_string(),
                    ));
                }
                self.send_ack(&message).await?;
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
                // TODO: Add missing message types (e.g. Channel messages)
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
        self.handle_recv_and_rtx_vectors(source, in_conn, recv_vec, rtx_vec)
            .await
    }

    async fn send_ack(&mut self, message: &Message) -> Result<(), SessionError> {
        if !self.send_acks {
            // nothing to do
            return Ok(());
        }

        // create ack and send it back to the source
        let src = message.get_source();
        let message_id = message.get_session_header().message_id;
        let slim_header = Some(SlimHeader::new(
            &self.local_name,
            &src,
            Some(SlimHeaderFlags::default().with_forward_to(message.get_incoming_conn())),
        ));

        let session_header = Some(SessionHeader::new(
            self.session_type.clone() as i32,
            slim_datapath::api::ProtoSessionMessageType::P2PAck.into(),
            message.get_session_header().session_id,
            message_id,
            &None,
            &None,
        ));

        let ack = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        self.tx.send_to_slim(Ok(ack)).await
    }

    async fn on_rtx_message(&mut self, message: Message) -> Result<(), SessionError> {
        // in case we get the and RTX reply the session must be relianle
        let source = message.get_source();
        let id = message.get_id();
        let in_conn = message.get_incoming_conn();

        debug!("received RTX reply for message {} from {}", id, source);

        // remote the timer
        let key = PendingRtxKey {
            name: source.clone(),
            id,
        };
        if let Some(mut pending) = self.pending_rtxs.remove(&key) {
            pending.timer.stop();
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
        self.handle_recv_and_rtx_vectors(source, in_conn, recv_vec, vec![])
            .await
    }

    async fn handle_recv_and_rtx_vectors(
        &mut self,
        source: Name,
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
                &source,
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

            let timer = self
                .timer_factory
                .as_ref()
                .unwrap()
                .create_and_start_timer(rtx_id, Some(source.clone()));

            let key = PendingRtxKey {
                name: source.clone(),
                id: rtx_id,
            };
            let val = PendingRtxVal {
                timer,
                message: rtx.clone(),
            };
            self.pending_rtxs.insert(key, val);

            // send message
            debug!("send rtx request for message {} to {}", rtx_id, source);
            self.tx.send_to_slim(Ok(rtx)).await?;
        }

        Ok(())
    }

    async fn on_timer_timeout(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!("timeout for message {} from {}", id, name);
        let key = PendingRtxKey { name, id };
        let pending = self.pending_rtxs.get(&key).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        debug!("send rtx request again");
        self.tx.send_to_slim(Ok(pending.message.clone())).await
    }

    async fn on_timer_failure(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!(
            "timer failure for message {} from {}, clear state",
            id, name
        );
        let key = PendingRtxKey { name, id };
        let mut pending = self.pending_rtxs.remove(&key).ok_or_else(|| {
            SessionError::Processing("missing pending rtx associated to timer".to_string())
        })?;

        // stop the timer and remove the name if no pending rtx left
        pending.timer.stop();

        // notify the application that the message was not delivered correctly
        self.tx
            .send_to_app(Err(SessionError::Processing(format!(
                "error receiving message {}. stop retrying",
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
            // Use a special timer ID for drain
            let drain_timer_id = DRAIN_TIMER_ID;
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
                            if message_id == DRAIN_TIMER_ID {
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
                            debug!("Drain message received for SessionReceiver");
                            self.start_drain(grace_period_ms);
                            if self.drain_timer.is_none() {
                                // close the sender
                                debug!("stop receiver loop");
                                break;
                            }
                        }
                        Some(_) => {
                            debug!("unexpected message type");
                        }
                        None => {
                            debug!("Channel closed, stopping SessionReceiver");
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) struct SessionReceiver<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    tx: Sender<SessionMessage>,
    // if true do not send signals
    drain: bool,
    _phantom: PhantomData<T>,
}

#[allow(dead_code)]
impl<T> SessionReceiver<T>
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

        let reliable_receiver = SessionReceiverInternal::new(
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

        SessionReceiver {
            tx,
            drain: false,
            _phantom: PhantomData,
        }
    }

    /// To be used to handle messages from slim
    pub async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // allow only RTX replies
        if self.drain {
            match message.get_session_message_type() {
                slim_datapath::api::ProtoSessionMessageType::RtxReply => {
                    // Allow RTX replies
                }
                _ => {
                    return Err(SessionError::Processing("sender is closing".to_string()));
                }
            }
        }
        self.tx
            .send(SessionMessage::OnMessage { message, direction })
            .await
            .map_err(|_| SessionError::Processing("Failed to queue message".to_string()))
    }

    /// Initiates a graceful drain of the receiver
    pub async fn drain(&mut self, grace_period_ms: u64) -> Result<(), SessionError> {
        if self.drain {
            return Err(SessionError::Processing("sender is closing".to_string()));
        }

        self.drain = true;
        self.tx
            .send(SessionMessage::Drain { grace_period_ms })
            .await
            .map_err(|_| SessionError::Processing("Failed to send drain message".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_receive_messages_1_and_2_sequentially() {
        // Test 1: receive messages 1 and 2, they should be correctly sent to the app
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Create test message 1
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Wait for the message to arrive at rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");

        // Verify the message was received correctly
        assert_eq!(received1.get_source(), remote_name);
        assert_eq!(received1.get_id(), 1);

        // Verify ack arriving at rx_slim
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack1.get_dst(), remote_name);
        assert_eq!(
            ack1.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 2
        let mut message2 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_2",
            vec![5, 6, 7, 8],
        );
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(10);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 2
        receiver
            .on_message(message2, MessageDirection::North)
            .await
            .expect("error sending message2");

        // Wait for the message to arrive at rx_app
        let received2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message2")
            .expect("channel closed")
            .expect("error in received message2");

        // Verify the message was received correctly
        assert_eq!(received2.get_source(), remote_name);
        assert_eq!(received2.get_id(), 2);

        // Verify ack arriving at rx_slim
        let ack2 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack2")
            .expect("channel closed")
            .expect("error in received ack2");

        // Verify the ack was sent correctly
        assert_eq!(ack2.get_dst(), remote_name);
        assert_eq!(
            ack2.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack2.get_session_header().get_message_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_loss_detection_with_rtx_timeout() {
        // Test 2: receive message 1 and 3, detect loss for message 2. RTX timer expires after retries
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Create test message 1
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Wait for message 1 to arrive at rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");

        assert_eq!(received1.get_id(), 1);

        // Verify ack arriving at rx_slim
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack1.get_dst(), remote_name);
        assert_eq!(
            ack1.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_3",
            vec![9, 10, 11, 12],
        );
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3, MessageDirection::North)
            .await
            .expect("error sending message3");

        // Verify ack arriving at rx_slim
        let ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack3.get_dst(), remote_name);
        assert_eq!(
            ack3.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Wait for RTX request to be sent to SLIM
        let rtx_request = timeout(Duration::from_millis(200), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request")
            .expect("channel closed")
            .expect("error in RTX request");

        // Verify it's an RTX request for message ID 2
        assert_eq!(
            rtx_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_request.get_id(), 2);
        assert_eq!(rtx_request.get_dst(), remote_name);

        // Wait for first RTX retry (after 500ms)
        let rtx_retry1 = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX retry1")
            .expect("channel closed")
            .expect("error in RTX retry1");

        // Verify it's the same RTX request
        assert_eq!(
            rtx_retry1.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_retry1.get_id(), 2);

        // Wait for second RTX retry
        let rtx_retry2 = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX retry2")
            .expect("channel closed")
            .expect("error in RTX retry2");

        // Verify it's the same RTX request
        assert_eq!(
            rtx_retry2.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_retry2.get_id(), 2);

        // After max retries, an error should be sent to the app
        let app_error = timeout(Duration::from_millis(800), rx_app.recv())
            .await
            .expect("timeout waiting for app error")
            .expect("channel closed");

        // Check that we received an error as expected
        match app_error {
            Err(SessionError::Processing(msg)) => {
                assert!(msg.contains("error receiving message 2. stop retrying"),);
            }
            _ => panic!(
                "Expected SessionError::Processing with max retries, got: {:?}",
                app_error
            ),
        }

        // No more RTX requests should be sent
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_reply_success() {
        // Test 3: same as test 2, but after the first rtx the receiver receives the rtx reply
        // so all messages 1, 2, and 3 are sent to the application
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Create test message 1
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Wait for message 1 to arrive at rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");

        assert_eq!(received1.get_id(), 1);

        // Verify ack arriving at rx_slim
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack1.get_dst(), remote_name);
        assert_eq!(
            ack1.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_3",
            vec![9, 10, 11, 12],
        );
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3, MessageDirection::North)
            .await
            .expect("error sending message3");

        // Verify ack arriving at rx_slim
        let ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack3.get_dst(), remote_name);
        assert_eq!(
            ack3.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Wait for RTX request to be sent to SLIM
        let rtx_request = timeout(Duration::from_millis(200), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request")
            .expect("channel closed")
            .expect("error in RTX request");

        // Verify it's an RTX request for message ID 2
        assert_eq!(
            rtx_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_request.get_id(), 2);

        // Create RTX reply with the missing message 2
        let mut rtx_reply = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_2",
            vec![5, 6, 7, 8],
        );
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send the RTX reply
        receiver
            .on_message(rtx_reply, MessageDirection::North)
            .await
            .expect("error sending rtx reply");

        // Now we should receive message 2 from the app (RTX reply success)
        let received2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message2")
            .expect("channel closed")
            .expect("error in received message2");

        assert_eq!(received2.get_id(), 2);

        // And then message 3 should also be delivered
        let received3 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message3")
            .expect("channel closed")
            .expect("error in received message3");

        assert_eq!(received3.get_id(), 3);

        // No more RTX requests should be sent since we got the reply
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // No errors should be sent to the app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_reply_with_error() {
        // Test 4: same as test 3, but the reply contains an error so the app should get
        // messages 1 and 3 plus an error for message 2
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Create test message 1
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Wait for message 1 to arrive at rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");

        assert_eq!(received1.get_id(), 1);

        // Verify ack arriving at rx_slim for message 1
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Verify the ack was sent correctly
        assert_eq!(ack1.get_dst(), remote_name);
        assert_eq!(
            ack1.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_3",
            vec![9, 10, 11, 12],
        );
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3, MessageDirection::North)
            .await
            .expect("error sending message3");

        // Verify ack arriving at rx_slim for message 3
        let ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack3")
            .expect("channel closed")
            .expect("error in received ack3");

        // Verify the ack was sent correctly
        assert_eq!(ack3.get_dst(), remote_name);
        assert_eq!(
            ack3.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PAck
        );
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Wait for RTX request to be sent to SLIM
        let rtx_request = timeout(Duration::from_millis(200), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request")
            .expect("channel closed")
            .expect("error in RTX request");

        // Verify it's an RTX request for message ID 2
        assert_eq!(
            rtx_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_request.get_id(), 2);

        // Create RTX reply with an error for message 2
        let mut rtx_reply = Message::new_publish(&remote_name, &local_name, None, "", vec![]);
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        // Set an error in the RTX reply
        rtx_reply.get_slim_header_mut().set_error(Some(true));

        // Send the RTX reply with error
        receiver
            .on_message(rtx_reply, MessageDirection::North)
            .await
            .expect("error sending rtx reply");

        // We should receive an error for the lost message
        let app_error = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for app error")
            .expect("channel closed");

        // Check that we received an error (None represents a lost message)
        match app_error {
            Err(SessionError::MessageLost(session_id)) => {
                assert_eq!(session_id, "10");
            }
            _ => panic!("Expected SessionError::MessageLost, got: {:?}", app_error),
        }

        // And then message 3 should be delivered
        let received3 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message3")
            .expect("channel closed")
            .expect("error in received message3");

        assert_eq!(received3.get_id(), 3);

        // No more RTX requests should be sent since we got the error reply
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // No more messages should be sent to the app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_senders_all_messages_delivered() {
        // Test 6: the receiver receives messages from 2 remote senders.
        // receives message 1 and 2 from remote1 and 1 and 2 from remote2.
        // all messages are correctly delivered to the app
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let group_name = Name::from_strings(["org", "ns", "group"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Create and send message 1 from remote1
        let mut message1_r1 = Message::new_publish(
            &remote1_name,
            &group_name,
            None,
            "payload_1_r1",
            vec![1, 2, 3, 4],
        );
        message1_r1
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1_r1.get_session_header_mut().set_message_id(1);
        message1_r1.get_session_header_mut().set_session_id(10);
        message1_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r1, MessageDirection::North)
            .await
            .expect("error sending message1_r1");

        // Create and send message 1 from remote2
        let mut message1_r2 = Message::new_publish(
            &remote2_name,
            &group_name,
            None,
            "payload_1_r2",
            vec![5, 6, 7, 8],
        );
        message1_r2
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1_r2.get_session_header_mut().set_message_id(1);
        message1_r2.get_session_header_mut().set_session_id(10);
        message1_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r2, MessageDirection::North)
            .await
            .expect("error sending message1_r2");

        // Create and send message 2 from remote1
        let mut message2_r1 = Message::new_publish(
            &remote1_name,
            &group_name,
            None,
            "payload_2_r1",
            vec![9, 10, 11, 12],
        );
        message2_r1
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message2_r1.get_session_header_mut().set_message_id(2);
        message2_r1.get_session_header_mut().set_session_id(10);
        message2_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r1, MessageDirection::North)
            .await
            .expect("error sending message2_r1");

        // Create and send message 2 from remote2
        let mut message2_r2 = Message::new_publish(
            &remote2_name,
            &group_name,
            None,
            "payload_2_r2",
            vec![13, 14, 15, 16],
        );
        message2_r2
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message2_r2.get_session_header_mut().set_message_id(2);
        message2_r2.get_session_header_mut().set_session_id(10);
        message2_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r2, MessageDirection::North)
            .await
            .expect("error sending message2_r2");

        // Collect all received messages
        let mut received_messages = Vec::new();
        for _ in 0..4 {
            let received = timeout(Duration::from_millis(100), rx_app.recv())
                .await
                .expect("timeout waiting for message")
                .expect("channel closed")
                .expect("error in received message");
            received_messages.push((received.get_source(), received.get_id()));
        }

        // Verify all messages were received correctly
        // Messages should be delivered in order for each sender
        assert!(received_messages.contains(&(remote1_name.clone(), 1)));
        assert!(received_messages.contains(&(remote1_name.clone(), 2)));
        assert!(received_messages.contains(&(remote2_name.clone(), 1)));
        assert!(received_messages.contains(&(remote2_name.clone(), 2)));

        // Collect all received acks (should be 4 acks for the 4 messages)
        let mut received_acks = Vec::new();
        for _ in 0..4 {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for ack")
                .expect("channel closed")
                .expect("error in received ack");
            received_acks.push((ack.get_dst(), ack.get_session_header().get_message_id()));
        }

        // Verify all acks were sent correctly
        assert!(received_acks.contains(&(remote1_name.clone(), 1)));
        assert!(received_acks.contains(&(remote1_name.clone(), 2)));
        assert!(received_acks.contains(&(remote2_name.clone(), 1)));
        assert!(received_acks.contains(&(remote2_name.clone(), 2)));

        // No more messages should arrive
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_senders_with_rtx_recovery() {
        // Test 7: the receiver receives messages from 2 remote senders.
        // receives message 1, 2 and 3 from remote1, receives 1 and 3 from remote2.
        // we should see an rtx for message 2 from remote2. after this recv a rtx reply and deliver everything to the app
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let group_name = Name::from_strings(["org", "ns", "group"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Send messages 1, 2, 3 from remote1 (complete sequence)
        let mut message1_r1 = Message::new_publish(
            &remote1_name,
            &group_name,
            None,
            "payload_1_r1",
            vec![1, 2, 3, 4],
        );
        message1_r1
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1_r1.get_session_header_mut().set_message_id(1);
        message1_r1.get_session_header_mut().set_session_id(10);
        message1_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r1, MessageDirection::North)
            .await
            .expect("error sending message1_r1");

        let mut message2_r1 = Message::new_publish(
            &remote1_name,
            &group_name,
            None,
            "payload_2_r1",
            vec![5, 6, 7, 8],
        );
        message2_r1
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message2_r1.get_session_header_mut().set_message_id(2);
        message2_r1.get_session_header_mut().set_session_id(10);
        message2_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r1, MessageDirection::North)
            .await
            .expect("error sending message2_r1");

        let mut message3_r1 = Message::new_publish(
            &remote1_name,
            &group_name,
            None,
            "payload_3_r1",
            vec![9, 10, 11, 12],
        );
        message3_r1
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3_r1.get_session_header_mut().set_message_id(3);
        message3_r1.get_session_header_mut().set_session_id(10);
        message3_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3_r1, MessageDirection::North)
            .await
            .expect("error sending message3_r1");

        // Send messages 1 and 3 from remote2 (missing message 2)
        let mut message1_r2 = Message::new_publish(
            &remote2_name,
            &group_name,
            None,
            "payload_1_r2",
            vec![13, 14, 15, 16],
        );
        message1_r2
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1_r2.get_session_header_mut().set_message_id(1);
        message1_r2.get_session_header_mut().set_session_id(10);
        message1_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r2, MessageDirection::North)
            .await
            .expect("error sending message1_r2");

        let mut message3_r2 = Message::new_publish(
            &remote2_name,
            &group_name,
            None,
            "payload_3_r2",
            vec![17, 18, 19, 20],
        );
        message3_r2
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3_r2.get_session_header_mut().set_message_id(3);
        message3_r2.get_session_header_mut().set_session_id(10);
        message3_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3_r2, MessageDirection::North)
            .await
            .expect("error sending message3_r2");

        // Collect all acknowledgments (should be 5 acks for the 5 messages sent)
        let mut received_acks = Vec::new();
        for _ in 0..5 {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for ack")
                .expect("channel closed")
                .expect("error in received ack");
            received_acks.push((ack.get_dst(), ack.get_session_header().get_message_id()));
        }

        // Verify all acks were sent correctly
        assert!(received_acks.contains(&(remote1_name.clone(), 1)));
        assert!(received_acks.contains(&(remote1_name.clone(), 2)));
        assert!(received_acks.contains(&(remote1_name.clone(), 3)));
        assert!(received_acks.contains(&(remote2_name.clone(), 1)));
        assert!(received_acks.contains(&(remote2_name.clone(), 3)));

        // Collect messages delivered to app from remote1 (should be all 3)
        let mut remote1_messages = Vec::new();
        for _ in 0..3 {
            let received = timeout(Duration::from_millis(100), rx_app.recv())
                .await
                .expect("timeout waiting for remote1 message")
                .expect("channel closed")
                .expect("error in received message");
            if received.get_source() == remote1_name {
                remote1_messages.push(received.get_id());
            }
        }

        // Verify remote1 messages are delivered in order
        remote1_messages.sort();
        assert_eq!(remote1_messages, vec![1, 2, 3]);

        // Collect message from remote2 (should be message 1)
        let received_r2_1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for remote2 message 1")
            .expect("channel closed")
            .expect("error in received message");
        assert_eq!(received_r2_1.get_source(), remote2_name);
        assert_eq!(received_r2_1.get_id(), 1);

        // Wait for RTX request for missing message 2 from remote2
        let rtx_request = timeout(Duration::from_millis(200), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request")
            .expect("channel closed")
            .expect("error in RTX request");

        // Verify it's an RTX request for message ID 2 from remote2
        assert_eq!(
            rtx_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_request.get_id(), 2);
        assert_eq!(rtx_request.get_dst(), remote2_name);

        // Create and send RTX reply with the missing message 2 from remote2
        let mut rtx_reply = Message::new_publish(
            &remote2_name,
            &local_name,
            None,
            "payload_2_r2",
            vec![21, 22, 23, 24],
        );
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(rtx_reply, MessageDirection::North)
            .await
            .expect("error sending rtx reply");

        // Now we should receive messages 2 and 3 from remote2
        let received_r2_2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for remote2 message 2")
            .expect("channel closed")
            .expect("error in received message 2 from remote2");
        assert_eq!(received_r2_2.get_source(), remote2_name);
        assert_eq!(received_r2_2.get_id(), 2);

        let received_r2_3 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for remote2 message 3")
            .expect("channel closed")
            .expect("error in received message 3 from remote2");
        assert_eq!(received_r2_3.get_source(), remote2_name);
        assert_eq!(received_r2_3.get_id(), 3);

        // No more RTX requests should be sent
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // No more messages should be sent to the app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_drain_with_no_pending_rtx() {
        // Test drain when there are no pending RTX requests - should complete immediately
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Send a message first to ensure the receiver is working
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send the message
        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Wait for the message to arrive at rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");
        assert_eq!(received1.get_id(), 1);

        // Consume the ACK
        let _ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1");

        // Now initiate drain - should complete immediately since no pending RTX
        let drain_result = receiver.drain(1000).await;
        assert!(
            drain_result.is_ok(),
            "Drain should succeed: {:?}",
            drain_result
        );

        // Verify receiver stops accepting new messages
        let mut message2 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_2",
            vec![5, 6, 7, 8],
        );
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(10);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        // This should fail because receiver is draining
        let result = receiver.on_message(message2, MessageDirection::North).await;
        match result {
            Err(SessionError::Processing(msg)) => {
                assert!(msg.contains("sender is closing"));
            }
            _ => panic!("Expected drain error, got: {:?}", result),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_drain_with_pending_rtx() {
        // Test drain when there are pending RTX requests - should wait for grace period
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // Send message 1
        let mut message1 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_1",
            vec![1, 2, 3, 4],
        );
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1, MessageDirection::North)
            .await
            .expect("error sending message1");

        // Consume message 1 and its ACK
        let _received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .unwrap();
        let _ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .unwrap();

        // Send message 3 (missing message 2) to create pending RTX
        let mut message3 = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_3",
            vec![9, 10, 11, 12],
        );
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3, MessageDirection::North)
            .await
            .expect("error sending message3");

        // Consume ACK for message 3
        let _ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .unwrap();

        // Wait for RTX request to be sent
        let _rtx_request = timeout(Duration::from_millis(200), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request");

        // Now initiate drain - should start grace period because of pending RTX
        let drain_result = receiver.drain(300).await; // 300ms grace period
        assert!(
            drain_result.is_ok(),
            "Drain should succeed: {:?}",
            drain_result
        );

        // Send RTX reply during grace period
        let mut rtx_reply = Message::new_publish(
            &remote_name,
            &local_name,
            None,
            "test_payload_2",
            vec![5, 6, 7, 8],
        );
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(rtx_reply, MessageDirection::North)
            .await
            .expect("error sending rtx reply");

        // Should receive messages 2 and 3 from app
        let received2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message2")
            .expect("channel closed")
            .expect("error in received message2");
        assert_eq!(received2.get_id(), 2);

        let received3 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message3")
            .expect("channel closed")
            .expect("error in received message3");
        assert_eq!(received3.get_id(), 3);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_double_drain_call() {
        // Test calling drain twice should return an error
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            SessionType::PointToPoint,
            true,
            tx,
        );

        // First drain call should succeed
        let first_drain = receiver.drain(1000).await;
        assert!(
            first_drain.is_ok(),
            "First drain should succeed: {:?}",
            first_drain
        );

        // Second drain call should fail
        let second_drain = receiver.drain(1000).await;
        match second_drain {
            Err(SessionError::Processing(msg)) => {
                assert!(msg.contains("sender is closing"));
            }
            _ => panic!("Expected double drain error, got: {:?}", second_drain),
        }
    }
}
