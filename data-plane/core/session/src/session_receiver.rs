// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::utils::{PUBLISH_TO, TRUE_VAL};
use slim_datapath::{api::ProtoMessage as Message, messages::Name};
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::common::new_message_from_session_fields;
use crate::transmitter::SessionTransmitter;
use crate::{
    SessionError, Transmitter,
    common::SessionMessage,
    receiver_buffer::ReceiverBuffer,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
};

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

/// used a result in OnMessage function
#[derive(PartialEq, Clone)]
enum ReceiverDrainStatus {
    NotDraining,
    Initiated,
    Completed,
}

#[allow(dead_code)]
pub struct SessionReceiver {
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
    session_type: ProtoSessionType,

    /// send to slim/app
    tx: SessionTransmitter,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ReceiverDrainStatus,

    /// shutdown receive flag. if set no message is delivered to the app
    /// the receiver will simply send acks on message reception
    shutdown_receive: bool,
}

#[allow(dead_code)]
impl SessionReceiver {
    /// to create the timer factory and send rtx messages
    /// timer_settings and tx_timer must be not null
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        local_name: Name,
        session_type: ProtoSessionType,
        tx: SessionTransmitter,
        tx_signals: Option<Sender<SessionMessage>>,
        shutdown_receive: bool,
    ) -> Self {
        let factory = if let Some(settings) = timer_settings
            && let Some(tx) = tx_signals
        {
            Some(TimerFactory::new(settings, tx))
        } else {
            None
        };

        tracing::debug!(
           %shutdown_receive, "creating session receiver"
        );

        SessionReceiver {
            buffer: HashMap::new(),
            pending_rtxs: HashMap::new(),
            timer_factory: factory,
            session_id,
            session_type,
            local_name,
            tx,
            draining_state: ReceiverDrainStatus::NotDraining,
            shutdown_receive,
        }
    }

    pub async fn on_message(&mut self, mut message: Message) -> Result<(), SessionError> {
        if self.draining_state == ReceiverDrainStatus::Completed {
            return Err(SessionError::SessionDrainingDrop);
        }

        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::Msg => {
                debug!("received message");
                if self.draining_state == ReceiverDrainStatus::Initiated {
                    // draining period is started, do no accept any new message
                    return Err(SessionError::SessionDrainingDrop);
                }
                if self.session_type == ProtoSessionType::PointToPoint {
                    // if the session is point to point publish_to falls back
                    // to the standard publish function
                    message.remove_metadata(PUBLISH_TO);
                }

                self.send_ack(&message).await?;
                self.on_publish_message(message).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::RtxReply => {
                debug!("received rtx message");
                self.on_rtx_message(message).await?;
            }
            _ => {
                // TODO: Add missing message types (e.g. Channel messages)
                debug!("unexpected message type");
            }
        }
        Ok(())
    }

    pub async fn on_publish_message(&mut self, message: Message) -> Result<(), SessionError> {
        // if shutdown_receive is true we just ack the message and do not deliver it to the app
        if self.shutdown_receive {
            debug!(
                id = %message.get_id(),
                source = %message.get_source(),
                "receiver is disabled, do not deliver message to app",
            );
            return Ok(());
        }

        if self.timer_factory.is_none() || message.contains_metadata(PUBLISH_TO) {
            debug!(
                id = %message.get_id(),
                source = %message.get_source(),
                "received message, send it to the app without reordering",
            );
            return self.tx.send_to_app(Ok(message)).await;
        }

        let source = message.get_source();
        let in_conn = message.get_incoming_conn();
        let buffer = self.buffer.entry(source.clone()).or_default();

        let (recv_vec, rtx_vec) = buffer.on_received_message(message);
        self.handle_recv_and_rtx_vectors(source, in_conn, recv_vec, rtx_vec)
            .await
    }

    pub async fn send_ack(&mut self, message: &Message) -> Result<(), SessionError> {
        // we need to send an ack message only if the factory is not none
        // in this case the session is reliable and the producer is expecting acks
        if self.timer_factory.is_none() {
            return Ok(());
        }

        let publish_meta = message.contains_metadata(PUBLISH_TO).then(|| {
            std::iter::once((PUBLISH_TO.to_string(), TRUE_VAL.to_string()))
                .collect::<std::collections::HashMap<_, _>>()
        });

        let ack = new_message_from_session_fields(
            &self.local_name,
            &message.get_source(),
            message.get_incoming_conn(),
            false,
            self.session_type,
            slim_datapath::api::ProtoSessionMessageType::MsgAck,
            message.get_session_header().session_id,
            message.get_id(),
            publish_meta,
        )?;

        self.tx.send_to_slim(Ok(ack)).await
    }

    pub async fn on_rtx_message(&mut self, message: Message) -> Result<(), SessionError> {
        // in case we get the and RTX reply the session must be reliable
        let source = message.get_source();
        let id = message.get_id();
        let in_conn = message.get_incoming_conn();

        debug!(
            %id, %source,
            "received RTX reply");

        // remote the timer
        let key = PendingRtxKey {
            name: source.clone(),
            id,
        };
        if let Some(mut pending) = self.pending_rtxs.remove(&key) {
            pending.timer.stop();
        }

        // if rtx is not an error pass to on_publish_message
        // otherwise manage the message loss
        if message.get_error().is_none() {
            return self.on_publish_message(message).await;
        }

        let buffer = self
            .buffer
            .get_mut(&source)
            .ok_or_else(|| SessionError::MissingPayload {
                context: "receiver_buffer_rtx_reply",
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
                        id = %r.get_id(),
                        source = %r.get_source(),
                        "received message, send it to the app",
                    );
                    self.tx.send_to_app(Ok(r)).await?;
                }
                None => {
                    debug!(
                        session_id = %self.session_id,
                        source = %source,
                        "lost message"
                    );
                    self.tx
                        .send_to_app(Err(SessionError::MessageLost(self.session_id)))
                        .await?;
                }
            }
        }

        for rtx_id in rtx_vec {
            debug!(
                id = %rtx_id,
                source = %source,
                "send rtx");

            let rtx = new_message_from_session_fields(
                &self.local_name,
                &source,
                in_conn,
                false,
                self.session_type,
                slim_datapath::api::ProtoSessionMessageType::RtxRequest,
                self.session_id,
                rtx_id,
                None,
            )?;

            // for each RTX start a timer
            debug!(id = %rtx_id,
            source = %source,"create rtx timer");

            let timer = self.timer_factory.as_ref().unwrap().create_and_start_timer(
                rtx_id,
                slim_datapath::api::ProtoSessionMessageType::RtxRequest,
                Some(source.clone()),
            );

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
            debug!(id = %rtx_id,
            source = %source, "send rtx request for message");
            self.tx.send_to_slim(Ok(rtx)).await?;
        }

        Ok(())
    }

    pub async fn on_timer_timeout(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!(%id, %name, "timeout for message");
        let key = PendingRtxKey { name, id };
        let pending = self
            .pending_rtxs
            .get(&key)
            .ok_or_else(|| SessionError::MissingPayload {
                context: "pending_rtx_timer",
            })?;

        debug!(%id, "send rtx request again");
        self.tx.send_to_slim(Ok(pending.message.clone())).await
    }

    pub async fn on_timer_failure(&mut self, id: u32, name: Name) -> Result<(), SessionError> {
        debug!(
            %id, %name,
            "timer failure for message, clear state",
        );
        let key = PendingRtxKey { name, id };
        let mut pending =
            self.pending_rtxs
                .remove(&key)
                .ok_or_else(|| SessionError::MissingPayload {
                    context: "pending_rtx_timer",
                })?;

        // stop the timer and remove the name if no pending rtx left
        pending.timer.stop();

        // notify the application that the message was not delivered correctly
        self.tx
            .send_to_app(Err(SessionError::receive_retry_failed(id)))
            .await
    }

    pub fn remove_endpoint(&mut self, endpoint: &Name) {
        // remove the buffer related to an endpoint so that if it is added again
        // the messages will not be dropped as duplicated
        tracing::debug!(%endpoint, "remove endpoint on the receiver");
        self.buffer.remove(endpoint);
    }

    pub fn start_drain(&mut self) {
        self.draining_state = ReceiverDrainStatus::Initiated;
        if self.pending_rtxs.is_empty() {
            self.draining_state = ReceiverDrainStatus::Completed;
        }
    }

    pub fn drain_completed(&self) -> bool {
        // Drain is complete if we're draining and no pending rtx remain
        if self.draining_state == ReceiverDrainStatus::Completed
            || self.draining_state == ReceiverDrainStatus::Initiated && self.pending_rtxs.is_empty()
        {
            return true;
        }
        false
    }

    pub fn close(&mut self) {
        for (_, mut p) in self.pending_rtxs.drain() {
            p.timer.stop();
        }
        self.pending_rtxs.clear();
        self.draining_state = ReceiverDrainStatus::Completed;
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Create test message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 2
        let mut message2 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_2", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(10);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 2
        receiver
            .on_message(message2)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack2.get_session_header().get_message_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_loss_detection_with_rtx_timeout() {
        // Test 2: receive message 1 and 3, detect loss for message 2. RTX timer expires after retries
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Create test message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_2", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_3", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Wait for the timer for rtx to be triggered and received at rx_signal
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer to be triggered")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type: _,
                name,
                timeouts: _,
            } => {
                receiver
                    .on_timer_timeout(message_id, name.unwrap())
                    .await
                    .expect("error sending rtx");
            }
            _ => panic!("received unexpected message"),
        }

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

        // Wait for the timer for rtx to be triggered and received at rx_signal
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer to be triggered")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type: _,
                name,
                timeouts: _,
            } => {
                receiver
                    .on_timer_timeout(message_id, name.unwrap())
                    .await
                    .expect("error sending rtx");
            }
            _ => panic!("received unexpected message"),
        }

        // Wait for first RTX retry
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

        // Wait for the timer for rtx to be triggered and received at rx_signal
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer to be triggered")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerFailure {
                message_id,
                message_type: _,
                name,
                timeouts: _,
            } => {
                receiver
                    .on_timer_failure(message_id, name.unwrap())
                    .await
                    .expect("error sending rtx");
            }
            _ => panic!("received unexpected message"),
        }

        // Wait for second RTX retry (sent during second timeout before failure)
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
            Err(SessionError::MessageReceiveRetryFailed { id }) => {
                assert_eq!(id, 2, "Expected retry failure for message id 2");
            }
            _ => panic!(
                "Expected SessionError::MessageReceiveRetryFailed with id 2, got: {:?}",
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Create test message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_3", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3)
            .await
            .expect("error sending message3");

        // Verify ack arriving at rx_slim
        let ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack3")
            .expect("channel closed")
            .expect("error in received ack3");

        // Verify the ack was sent correctly
        assert_eq!(ack3.get_dst(), remote_name);
        assert_eq!(
            ack3.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MsgAck
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
        let mut rtx_reply = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_2", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send the RTX reply
        receiver
            .on_message(rtx_reply)
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Create test message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 1
        receiver
            .on_message(message1)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);

        // Create test message 3 (message 2 is missing)
        let mut message3 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_3", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send message 3 (this should trigger RTX request for message 2)
        receiver
            .on_message(message3)
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
            slim_datapath::api::ProtoSessionMessageType::MsgAck
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
        let mut rtx_reply = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        // Set an error in the RTX reply
        rtx_reply.get_slim_header_mut().set_error(Some(true));

        // Send the RTX reply with error
        receiver
            .on_message(rtx_reply)
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
                assert_eq!(session_id, 10);
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let group_name = Name::from_strings(["org", "ns", "group"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Create and send message 1 from remote1
        let mut message1_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_1_r1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r1.get_session_header_mut().set_message_id(1);
        message1_r1.get_session_header_mut().set_session_id(10);
        message1_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r1)
            .await
            .expect("error sending message1_r1");

        // Create and send message 1 from remote2
        let mut message1_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_1_r2", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        message1_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r2.get_session_header_mut().set_message_id(1);
        message1_r2.get_session_header_mut().set_session_id(10);
        message1_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r2)
            .await
            .expect("error sending message1_r2");

        // Create and send message 2 from remote1
        let mut message2_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_2_r1", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message2_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2_r1.get_session_header_mut().set_message_id(2);
        message2_r1.get_session_header_mut().set_session_id(10);
        message2_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r1)
            .await
            .expect("error sending message2_r1");

        // Create and send message 2 from remote2
        let mut message2_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_2_r2", vec![13, 14, 15, 16])
            .build_publish()
            .unwrap();
        message2_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2_r2.get_session_header_mut().set_message_id(2);
        message2_r2.get_session_header_mut().set_session_id(10);
        message2_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r2)
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let group_name = Name::from_strings(["org", "ns", "group"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Send messages 1, 2, 3 from remote1 (complete sequence)
        let mut message1_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_1_r1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r1.get_session_header_mut().set_message_id(1);
        message1_r1.get_session_header_mut().set_session_id(10);
        message1_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r1)
            .await
            .expect("error sending message1_r1");

        let mut message2_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_2_r1", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        message2_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2_r1.get_session_header_mut().set_message_id(2);
        message2_r1.get_session_header_mut().set_session_id(10);
        message2_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r1)
            .await
            .expect("error sending message2_r1");

        let mut message3_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_3_r1", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message3_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3_r1.get_session_header_mut().set_message_id(3);
        message3_r1.get_session_header_mut().set_session_id(10);
        message3_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3_r1)
            .await
            .expect("error sending message3_r1");

        // Send messages 1 and 3 from remote2 (missing message 2)
        let mut message1_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_1_r2", vec![13, 14, 15, 16])
            .build_publish()
            .unwrap();
        message1_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r2.get_session_header_mut().set_message_id(1);
        message1_r2.get_session_header_mut().set_session_id(10);
        message1_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r2)
            .await
            .expect("error sending message1_r2");

        let mut message3_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(group_name.clone())
            .application_payload("payload_3_r2", vec![17, 18, 19, 20])
            .build_publish()
            .unwrap();
        message3_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3_r2.get_session_header_mut().set_message_id(3);
        message3_r2.get_session_header_mut().set_session_id(10);
        message3_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3_r2)
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
        let mut rtx_reply = Message::builder()
            .source(remote2_name.clone())
            .destination(local_name.clone())
            .application_payload("payload_2_r2", vec![21, 22, 23, 24])
            .build_publish()
            .unwrap();
        rtx_reply.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);
        rtx_reply.get_session_header_mut().set_message_id(2);
        rtx_reply.get_session_header_mut().set_session_id(10);
        rtx_reply.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(rtx_reply)
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
    async fn test_publish_to_message_bypasses_reordering() {
        // Test that messages with PUBLISH_TO metadata are sent directly to the app without buffering/reordering
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );

        // Send message 3 with PUBLISH_TO (out of order)
        let mut message3 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_3", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));
        message3
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message3)
            .await
            .expect("error sending message3");

        // Message 3 should be delivered immediately to app (bypassing reordering)
        let received = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error in received message");

        assert_eq!(received.get_id(), 3);
        assert_eq!(received.get_source(), remote_name);

        // Verify ack has PUBLISH_TO metadata
        let ack = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack")
            .expect("channel closed")
            .expect("error in received ack");

        assert_eq!(ack.get_dst(), remote_name);
        assert!(ack.metadata.contains_key(PUBLISH_TO));
        assert_eq!(
            ack.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_p2p_session_receiver() {
        // Test that PUBLISH_TO metadata is removed in point-to-point sessions on receiver side
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );

        // Send message with PUBLISH_TO in a P2P session
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));
        message1
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message1)
            .await
            .expect("error sending message");

        // Message should be delivered to app
        let received = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error in received message");

        assert_eq!(received.get_id(), 1);

        // Ack should NOT have PUBLISH_TO metadata (since it was removed in P2P mode)
        let ack = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack")
            .expect("channel closed")
            .expect("error in received ack");

        assert_eq!(ack.get_dst(), remote_name);
        assert!(!ack.metadata.contains_key(PUBLISH_TO));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_mixed_with_normal_messages() {
        // Test receiving both normal and PUBLISH_TO messages in a multicast session
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );

        // Send normal message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("normal_1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1)
            .await
            .expect("error sending message1");

        // Should receive message 1
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");
        assert_eq!(received1.get_id(), 1);

        // Get ack for message 1 (no PUBLISH_TO)
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");
        assert!(!ack1.metadata.contains_key(PUBLISH_TO));

        // Send PUBLISH_TO message with ID 100 (out of normal sequence)
        let mut message_pt = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("publish_to", vec![100])
            .build_publish()
            .unwrap();
        message_pt.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message_pt.get_session_header_mut().set_message_id(100);
        message_pt.get_session_header_mut().set_session_id(10);
        message_pt.get_slim_header_mut().set_incoming_conn(Some(1));
        message_pt
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message_pt)
            .await
            .expect("error sending publish_to message");

        // PUBLISH_TO message should be delivered immediately
        let received_pt = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for publish_to message")
            .expect("channel closed")
            .expect("error in received publish_to message");
        assert_eq!(received_pt.get_id(), 100);

        // Get ack for PUBLISH_TO message (should have PUBLISH_TO metadata)
        let ack_pt = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack publish_to")
            .expect("channel closed")
            .expect("error in received ack publish_to");
        assert!(ack_pt.metadata.contains_key(PUBLISH_TO));

        // Send normal message 2
        let mut message2 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("normal_2", vec![5, 6, 7, 8])
            .build_publish()
            .unwrap();
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(10);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2)
            .await
            .expect("error sending message2");

        // Should receive message 2
        let received2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message2")
            .expect("channel closed")
            .expect("error in received message2");
        assert_eq!(received2.get_id(), 2);

        // Get ack for message 2 (no PUBLISH_TO)
        let ack2 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack2")
            .expect("channel closed")
            .expect("error in received ack2");
        assert!(!ack2.metadata.contains_key(PUBLISH_TO));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_no_buffering_unreliable_mode() {
        // Test that in unreliable mode (no timer factory), PUBLISH_TO messages work correctly
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            None, // No timer settings = unreliable mode
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            None,
            false,
        );

        // Send message with PUBLISH_TO
        let mut message = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message.get_session_header_mut().set_message_id(1);
        message.get_session_header_mut().set_session_id(10);
        message.get_slim_header_mut().set_incoming_conn(Some(1));
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message)
            .await
            .expect("error sending message");

        // Message should be delivered immediately to app
        let received = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error in received message");

        assert_eq!(received.get_id(), 1);

        // No ack should be sent in unreliable mode
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no ack in unreliable mode but got: {:?}",
            res
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_out_of_order_delivery() {
        // Test that PUBLISH_TO messages are delivered immediately even when out of order
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );

        // Send message 1 normally
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("normal_1", vec![1])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1)
            .await
            .expect("error sending message1");

        // Receive message 1
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1")
            .expect("channel closed")
            .expect("error in received message1");
        assert_eq!(received1.get_id(), 1);

        // Clear ack
        let _ = rx_slim.recv().await;

        // Send message 5 with PUBLISH_TO (skipping 2, 3, 4)
        let mut message5_pt = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("publish_to_5", vec![5])
            .build_publish()
            .unwrap();
        message5_pt.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message5_pt.get_session_header_mut().set_message_id(5);
        message5_pt.get_session_header_mut().set_session_id(10);
        message5_pt.get_slim_header_mut().set_incoming_conn(Some(1));
        message5_pt
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message5_pt)
            .await
            .expect("error sending message5_pt");

        // Message 5 should be delivered immediately (not buffered waiting for 2, 3, 4)
        let received5 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message5")
            .expect("channel closed")
            .expect("error in received message5");
        assert_eq!(received5.get_id(), 5);

        // Clear ack
        let _ = rx_slim.recv().await;

        // Send message 3 normally (still missing 2)
        let mut message3 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("normal_3", vec![3])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(10);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message3)
            .await
            .expect("error sending message3");

        // Message 3 should NOT be delivered yet (waiting for message 2)
        // We should see an RTX request for message 2
        let rtx_request = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for rtx or ack")
            .expect("channel closed")
            .expect("error in rtx request");

        // Should be RTX for message 2 or ack for message 3
        if rtx_request.get_session_message_type()
            == slim_datapath::api::ProtoSessionMessageType::MsgAck
        {
            // It's the ack for message 3, now wait for RTX
            let rtx = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for rtx")
                .expect("channel closed")
                .expect("error in rtx");
            assert_eq!(
                rtx.get_session_message_type(),
                slim_datapath::api::ProtoSessionMessageType::RtxRequest
            );
            assert_eq!(rtx.get_id(), 2);
        } else {
            assert_eq!(
                rtx_request.get_session_message_type(),
                slim_datapath::api::ProtoSessionMessageType::RtxRequest
            );
            assert_eq!(rtx_request.get_id(), 2);
        }

        // Message 3 should still not be delivered to app yet
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Message 3 should not be delivered yet, but got: {:?}",
            res
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_publish_to_messages_from_different_senders() {
        // Test receiving PUBLISH_TO messages from multiple senders in multicast session
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );

        // Send PUBLISH_TO message from remote1
        let mut message_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(local_name.clone())
            .application_payload("pt_r1", vec![1])
            .build_publish()
            .unwrap();
        message_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message_r1.get_session_header_mut().set_message_id(100);
        message_r1.get_session_header_mut().set_session_id(10);
        message_r1.get_slim_header_mut().set_incoming_conn(Some(1));
        message_r1
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message_r1)
            .await
            .expect("error sending message from remote1");

        // Send PUBLISH_TO message from remote2
        let mut message_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(local_name.clone())
            .application_payload("pt_r2", vec![2])
            .build_publish()
            .unwrap();
        message_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message_r2.get_session_header_mut().set_message_id(200);
        message_r2.get_session_header_mut().set_session_id(10);
        message_r2.get_slim_header_mut().set_incoming_conn(Some(1));
        message_r2
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message_r2)
            .await
            .expect("error sending message from remote2");

        // Both messages should be delivered immediately
        let mut received_messages = Vec::new();
        for _ in 0..2 {
            let received = timeout(Duration::from_millis(100), rx_app.recv())
                .await
                .expect("timeout waiting for message")
                .expect("channel closed")
                .expect("error in received message");
            received_messages.push((received.get_source(), received.get_id()));
        }

        assert!(received_messages.contains(&(remote1_name.clone(), 100)));
        assert!(received_messages.contains(&(remote2_name.clone(), 200)));

        // Both acks should have PUBLISH_TO metadata
        for _ in 0..2 {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for ack")
                .expect("channel closed")
                .expect("error in received ack");
            assert!(ack.metadata.contains_key(PUBLISH_TO));
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_receive_out_of_order_messages() {
        // Test 3: Send messages 1, 3, 2 out of order with shutdown_receive=true.
        // Verify all ACKs are sent but no messages reach the app, and no RTX requests are generated.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            true, // shutdown_receive enabled
        );

        // Send messages out of order: 1, 3, 2
        let message_order = [1u32, 3u32, 2u32];
        for msg_id in message_order.iter() {
            let mut message = Message::builder()
                .source(remote_name.clone())
                .destination(local_name.clone())
                .application_payload(
                    &format!("test_payload_{}", msg_id),
                    vec![*msg_id as u8, (*msg_id + 1) as u8],
                )
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
            message.get_session_header_mut().set_message_id(*msg_id);
            message.get_session_header_mut().set_session_id(10);
            message.get_slim_header_mut().set_incoming_conn(Some(1));

            receiver
                .on_message(message)
                .await
                .unwrap_or_else(|_| panic!("error sending message{}", msg_id));
        }

        // Verify all 3 ACKs were sent (in the order messages were received: 1, 3, 2)
        let expected_order = [1u32, 3u32, 2u32];
        for expected_id in expected_order {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for ack{}", expected_id))
                .expect("channel closed")
                .unwrap_or_else(|_| panic!("error in received ack{}", expected_id));

            assert_eq!(ack.get_dst(), remote_name);
            assert_eq!(
                ack.get_session_message_type(),
                slim_datapath::api::ProtoSessionMessageType::MsgAck
            );
            assert_eq!(ack.get_session_header().get_message_id(), expected_id);
        }

        // Verify no messages were delivered to app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to app but got: {:?}",
            res
        );

        // Verify no RTX requests were sent (even though messages arrived out of order)
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected no RTX requests but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_receive_multiple_senders() {
        // Test 4: Two senders send messages simultaneously with shutdown_receive=true.
        // Verify ACKs go to both senders but no messages reach the app.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote1_name = Name::from_strings(["org", "ns", "remote1"]);
        let remote2_name = Name::from_strings(["org", "ns", "remote2"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            true, // shutdown_receive enabled
        );

        // Send message 1 from remote1
        let mut message1_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_r1_1", vec![1, 2, 3])
            .build_publish()
            .unwrap();
        message1_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r1.get_session_header_mut().set_message_id(1);
        message1_r1.get_session_header_mut().set_session_id(10);
        message1_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r1)
            .await
            .expect("error sending message1 from remote1");

        // Send message 1 from remote2
        let mut message1_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_r2_1", vec![4, 5, 6])
            .build_publish()
            .unwrap();
        message1_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1_r2.get_session_header_mut().set_message_id(1);
        message1_r2.get_session_header_mut().set_session_id(10);
        message1_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1_r2)
            .await
            .expect("error sending message1 from remote2");

        // Send message 2 from remote1
        let mut message2_r1 = Message::builder()
            .source(remote1_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_r1_2", vec![7, 8, 9])
            .build_publish()
            .unwrap();
        message2_r1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2_r1.get_session_header_mut().set_message_id(2);
        message2_r1.get_session_header_mut().set_session_id(10);
        message2_r1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r1)
            .await
            .expect("error sending message2 from remote1");

        // Send message 2 from remote2
        let mut message2_r2 = Message::builder()
            .source(remote2_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_r2_2", vec![10, 11, 12])
            .build_publish()
            .unwrap();
        message2_r2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2_r2.get_session_header_mut().set_message_id(2);
        message2_r2.get_session_header_mut().set_session_id(10);
        message2_r2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2_r2)
            .await
            .expect("error sending message2 from remote2");

        // Collect all 4 ACKs
        let mut received_acks = Vec::new();
        for _ in 0..4 {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for ack")
                .expect("channel closed")
                .expect("error in received ack");
            received_acks.push((ack.get_dst(), ack.get_session_header().get_message_id()));
        }

        // Verify all ACKs were sent to correct destinations
        assert!(received_acks.contains(&(remote1_name.clone(), 1)));
        assert!(received_acks.contains(&(remote1_name.clone(), 2)));
        assert!(received_acks.contains(&(remote2_name.clone(), 1)));
        assert!(received_acks.contains(&(remote2_name.clone(), 2)));

        // Verify no messages were delivered to app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to app but got: {:?}",
            res
        );

        // Verify no RTX requests were sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected no RTX requests but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_receive_publish_to_messages() {
        // Test 5: Verify that even PUBLISH_TO messages (which normally bypass buffering)
        // are not delivered to the app when shutdown_receive=true, but still generate ACKs.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            true, // shutdown_receive enabled
        );

        // Send normal message 1
        let mut message1 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_1", vec![1, 2, 3])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(10);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message1)
            .await
            .expect("error sending message1");

        // Send PUBLISH_TO message (ID 100)
        let mut message_pt = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_publish_to", vec![100, 101, 102])
            .build_publish()
            .unwrap();
        message_pt.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message_pt.get_session_header_mut().set_message_id(100);
        message_pt.get_session_header_mut().set_session_id(10);
        message_pt.get_slim_header_mut().set_incoming_conn(Some(1));
        message_pt
            .metadata
            .insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        receiver
            .on_message(message_pt)
            .await
            .expect("error sending publish_to message");

        // Send normal message 2
        let mut message2 = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("test_payload_2", vec![4, 5, 6])
            .build_publish()
            .unwrap();
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(10);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        receiver
            .on_message(message2)
            .await
            .expect("error sending message2");

        // Verify all 3 ACKs were sent (including PUBLISH_TO)
        let mut received_acks = Vec::new();
        for _ in 0..3 {
            let ack = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for ack")
                .expect("channel closed")
                .expect("error in received ack");

            let has_publish_to = ack.metadata.contains_key(PUBLISH_TO);
            received_acks.push((ack.get_session_header().get_message_id(), has_publish_to));
        }

        // Verify ACKs for message 1, 100 (with PUBLISH_TO), and 2
        assert!(received_acks.contains(&(1, false)));
        assert!(received_acks.contains(&(100, true))); // PUBLISH_TO message should have metadata in ACK
        assert!(received_acks.contains(&(2, false)));

        // Verify no messages were delivered to app (including PUBLISH_TO message)
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to app (including PUBLISH_TO) but got: {:?}",
            res
        );

        // Verify no RTX requests were sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected no RTX requests but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_receive_unreliable_mode() {
        // Test: Verify shutdown_receive works in unreliable mode (no timer_factory, no ACKs)
        // Messages should be dropped without being delivered to app.
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            None, // No timer settings = unreliable mode
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            tx,
            None, // No signal channel in unreliable mode
            true, // shutdown_receive enabled
        );

        // Send messages 1, 2, 3
        for msg_id in 1..=3 {
            let mut message = Message::builder()
                .source(remote_name.clone())
                .destination(local_name.clone())
                .application_payload(
                    &format!("test_payload_{}", msg_id),
                    vec![msg_id as u8, (msg_id + 1) as u8],
                )
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
            message
                .get_session_header_mut()
                .set_message_id(msg_id as u32);
            message.get_session_header_mut().set_session_id(10);
            message.get_slim_header_mut().set_incoming_conn(Some(1));

            receiver
                .on_message(message)
                .await
                .unwrap_or_else(|_| panic!("error sending message{}", msg_id));
        }

        // In unreliable mode with shutdown_receive, no ACKs should be sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no ACKs in unreliable mode but got: {:?}",
            res
        );

        // Verify no messages were delivered to app
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to app but got: {:?}",
            res
        );

        // Verify buffer is not used (unreliable mode doesn't use buffer)
        assert!(
            receiver.buffer.is_empty(),
            "Expected empty buffer in unreliable mode"
        );
    }
}
