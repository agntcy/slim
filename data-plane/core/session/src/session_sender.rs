// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use rand::Rng;
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::utils::{MAX_PUBLISH_ID, PUBLISH_TO};
use slim_datapath::{api::ProtoMessage as Message, messages::Name};
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tracing::debug;

use crate::common::new_message_from_session_fields;
use crate::transmitter::SessionTransmitter;
use crate::{
    SessionError, Transmitter,
    common::SessionMessage,
    producer_buffer::ProducerBuffer,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
};

/// used a result in OnMessage function
#[derive(PartialEq, Clone)]
enum SenderDrainStatus {
    NotDraining,
    Initiated,
    Completed,
}

#[allow(dead_code)]
struct GroupTimer {
    /// list of names for which we did not get an ack yet
    missing_timers: HashSet<Name>,

    /// the timer
    timer: Timer,
}

#[allow(dead_code)]
pub struct SessionSender {
    /// buffer storing messages coming from the application
    buffer: ProducerBuffer,

    /// timer factory to crate timers for acks
    timer_factory: Option<TimerFactory>,

    /// list of pending acks for each message
    /// Message sent to the group are stored in the buffer and there
    /// is no need to store them elsewhere. The message field will be
    /// set to None in this case. Instead messages sent to a particolar
    /// endpoint (using the publish_to) need to be stored in the map
    pending_acks: HashMap<u32, (GroupTimer, Option<Message>)>,

    /// list of timer ids associated for each endpoint
    /// this list do not ta
    pending_acks_per_endpoint: HashMap<Name, HashSet<u32>>,

    /// list of the endpoints in the conversation
    /// on message send, create an ack timer for each endpoint in the list
    /// on endpoint remove, delete all the acks to the endpoint
    endpoints_list: HashSet<Name>,

    /// message id, used to send sequential messages
    /// it goes from 0 to 2^16. higher ids are used for messages
    /// sent using the publish_to function.
    next_id: u32,

    /// session id to had in the message header
    session_id: u32,

    /// session type to set in the message header
    session_type: ProtoSessionType,

    /// send packets to slim or the app
    tx: SessionTransmitter,

    /// set to true if the sender is receiving packets
    /// to send but no one is connected to the session yet
    to_flush: bool,

    /// drain state - when true, no new messages from app are accepted
    draining_state: SenderDrainStatus,

    /// oneshot senders to signal when network acks are received for each message
    ack_notifiers: HashMap<u32, oneshot::Sender<Result<(), SessionError>>>,

    // shutdown_send flag. if set no message is sent to the network
    // and all messages are dropped
    shutdown_send: bool,
}

#[allow(dead_code)]
impl SessionSender {
    const MAX_FANOUT: u32 = 256;

    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        session_type: ProtoSessionType,
        tx: SessionTransmitter,
        tx_signals: Option<Sender<SessionMessage>>,
        shutdown_send: bool,
    ) -> Self {
        let factory = if let Some(settings) = timer_settings
            && let Some(tx) = tx_signals
        {
            Some(TimerFactory::new(settings, tx))
        } else {
            None
        };

        debug!(
           %shutdown_send, "creating session sender"
        );

        SessionSender {
            buffer: ProducerBuffer::with_capacity(512),
            timer_factory: factory,
            pending_acks: HashMap::new(),
            pending_acks_per_endpoint: HashMap::new(),
            endpoints_list: HashSet::new(),
            next_id: 0,
            session_id,
            session_type,
            tx,
            to_flush: false,
            draining_state: SenderDrainStatus::NotDraining,
            ack_notifiers: HashMap::new(),
            shutdown_send,
        }
    }

    /// Send a message with optional acknowledgment notification
    pub async fn on_message(
        &mut self,
        mut message: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        if self.draining_state == SenderDrainStatus::Completed {
            if let Some(tx) = ack_tx {
                let _ = tx.send(Err(SessionError::SessionDrainingDrop));
            }
            return Err(SessionError::SessionDrainingDrop);
        }

        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::Msg => {
                debug!("received message");
                if self.draining_state == SenderDrainStatus::Initiated {
                    if let Some(tx) = ack_tx {
                        let _ = tx.send(Err(SessionError::SessionDrainingDrop));
                    }
                    return Err(SessionError::SessionDrainingDrop);
                }
                if self.session_type == ProtoSessionType::PointToPoint {
                    // if the session is point to point publish_to falls back
                    // to the standard publish function
                    message.remove_metadata(PUBLISH_TO);
                }
                self.on_publish_message(message, ack_tx).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::MsgAck => {
                debug!("received ack message");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                if self.timer_factory.is_none() {
                    return Ok(());
                }
                self.on_ack_message(&message);
            }
            slim_datapath::api::ProtoSessionMessageType::RtxRequest => {
                debug!("received rtx message");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                if self.timer_factory.is_none() {
                    return Ok(());
                }
                self.on_ack_message(&message);
                self.on_rtx_message(message).await?;
            }
            _ => {
                debug!("unexpected message type");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
            }
        }

        Ok(())
    }

    async fn on_publish_message(
        &mut self,
        mut message: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        // if shutdown_send is true we drop all messages but we signal success
        // to the application using the ack channel if provided
        if self.shutdown_send {
            debug!(message_id = %message.get_id(), "sender is shutdown, drop message");
            if let Some(tx) = ack_tx {
                let _ = tx.send(Err(SessionError::SessionSenderShutdown));
            }
            return Err(SessionError::SessionSenderShutdown);
        }

        let is_publish_to = message.metadata.contains_key(PUBLISH_TO);

        // if is_publish_to and the message destination is not
        // in the endpoint list we drop the messages
        if is_publish_to && !self.endpoints_list.contains(&message.get_dst()) {
            debug!(
                dst = %message.get_dst(),
                "cannot forward the message to the select destination",
            );
            return Err(SessionError::UnknownDestination(message.get_dst()));
        }

        let (message_id, fanout) = self.id_and_fanout(is_publish_to);

        // Set the session id, message id and session type
        let session_header = message.get_session_header_mut();
        session_header.set_message_id(message_id);
        session_header.set_session_id(self.session_id);
        session_header.set_session_type(self.session_type);
        message.get_slim_header_mut().set_fanout(fanout);

        // Buffer the message after setting the ID (only for non-publish_to messages)
        if !is_publish_to {
            self.buffer.push(message.clone());
        }

        // Store the ack notifier if provided
        if let Some(tx) = ack_tx {
            // In unreliable mode (no timer_factory), signal success immediately
            // since we don't wait for network acks
            if self.timer_factory.is_none() {
                let _ = tx.send(Ok(()));
            } else {
                self.ack_notifiers.insert(message_id, tx);
            }
        }

        if self.endpoints_list.is_empty() {
            debug!(
                "there is no remote endopoint connected to the session, store the packet and send it later"
            );
            self.to_flush = true;
            return Ok(());
        }

        self.set_timer_and_send(message).await
    }

    fn id_and_fanout(&mut self, is_publish_to: bool) -> (u32, u32) {
        if is_publish_to {
            // if the message is sent using the publish_to the message
            // id is taken randomly and it must be in range [MAX_PUBLISH_ID + 1, u32::MAX - 1]
            let id = rand::rng().random_range((MAX_PUBLISH_ID + 1)..u32::MAX);
            return (id, 1);
        }

        // by increasing next_id before assign it to message_id
        // we always start the sequence from 1. The max message_id
        // is MAX_PUBLISH_ID. after that we wrap to 0.
        self.next_id = (self.next_id + 1) % (MAX_PUBLISH_ID + 1);
        let id = self.next_id;

        let fanout = match self.session_type {
            ProtoSessionType::Multicast => Self::MAX_FANOUT,
            _ => 1,
        };

        (id, fanout)
    }

    async fn set_timer_and_send(&mut self, message: Message) -> Result<(), SessionError> {
        let message_id = message.get_id();
        let is_publish_to = message.metadata.contains_key(PUBLISH_TO);
        debug!(%message_id, "send new message");

        if let Some(timer_factory) = &self.timer_factory {
            debug!("reliable sender, set all timers");
            // set the list of missing timers according to the destination of message
            let missing_timers = if is_publish_to {
                std::iter::once(message.get_dst()).collect::<HashSet<_>>()
            } else {
                self.endpoints_list.clone()
            };

            // create a timer for the new packet and update the state
            let gt = GroupTimer {
                missing_timers,
                timer: timer_factory.create_and_start_timer(
                    message_id,
                    message.get_session_message_type(),
                    None,
                ),
            };

            // insert in pending acks and get the endpoint_to_track
            let endpoints_to_track = if is_publish_to {
                self.pending_acks
                    .insert(message_id, (gt, Some(message.clone())));
                std::iter::once(message.get_dst()).collect::<Vec<_>>()
            } else {
                self.pending_acks.insert(message_id, (gt, None));
                self.endpoints_list.iter().cloned().collect::<Vec<_>>()
            };

            for n in &endpoints_to_track {
                debug!(%message_id, remote = %n, "add timer for message");
                if let Some(acks) = self.pending_acks_per_endpoint.get_mut(n) {
                    acks.insert(message_id);
                } else {
                    let mut set = HashSet::new();
                    set.insert(message_id);
                    self.pending_acks_per_endpoint.insert(n.clone(), set);
                }
            }
        }

        debug!("send message");
        // send the message
        self.tx.send_to_slim(Ok(message)).await
    }

    fn on_ack_message(&mut self, message: &Message) {
        let source = message.get_source();
        let message_id = message.get_id();
        debug!(%message_id, %source, "received ack message");

        let mut delete = false;
        // remove the source from the pending acks
        // notice that the state for this ack id may not exist if all the endpoints
        // associated to it where removed before getting the acks
        if let Some((gt, _m)) = self.pending_acks.get_mut(&message_id) {
            debug!(%source, "try to remove from pending acks");
            gt.missing_timers.remove(&source);
            if gt.missing_timers.is_empty() {
                debug!("all acks received, remove timer");
                // all acks received. stop the timer and remove the entry
                gt.timer.stop();
                delete = true;

                // Signal success to the ack notifier if present
                if let Some(tx) = self.ack_notifiers.remove(&message_id) {
                    let _ = tx.send(Ok(()));
                }
            }
        }

        // remove the pending ack from the name
        // also here the endpoint may not exists anymore
        if let Some(set) = self.pending_acks_per_endpoint.get_mut(&source) {
            debug!(
                %message_id, %source,
                "remove message from pending acks for"
            );
            // here we do not remove the name even if the set is empty
            // remove it only if endpoint is deleted
            set.remove(&message_id);
        }

        // all acks received for this timer, remove it
        if delete {
            debug!(
                id = %message_id,
                "all acks received, remove timer",
            );
            self.pending_acks.remove(&message_id);
        }
    }

    async fn on_rtx_message(&mut self, message: Message) -> Result<(), SessionError> {
        let source = message.get_source();
        let message_id = message.get_id();
        let incoming_conn = message.get_incoming_conn();

        debug!(
            id = %message_id,
            %source,
            "received rtx request",
        );

        // try to get the required message from the buffer
        if let Some(mut msg) = self.buffer.get(message_id as usize) {
            // the message still exists, send it back as a reply for the RTX
            debug!("the message is still exists, send it as rtx reply");
            msg.get_slim_header_mut().set_destination(&source);
            msg.get_slim_header_mut()
                .set_forward_to(Some(incoming_conn));
            msg.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);

            // send the message
            self.tx.send_to_slim(Ok(msg)).await
        } else {
            debug!("the message does not exists anymore, send and error");
            let msg = new_message_from_session_fields(
                &message.get_dst(),
                &message.get_source(),
                incoming_conn,
                true,
                message.get_session_type(),
                slim_datapath::api::ProtoSessionMessageType::RtxReply,
                self.session_id,
                message_id,
                None,
            )?;

            // send the message
            self.tx.send_to_slim(Ok(msg)).await
        }
    }

    pub async fn on_timer_timeout(&mut self, id: u32) -> Result<(), SessionError> {
        debug!(%id, "message timeout");

        if id > MAX_PUBLISH_ID {
            // this must be a message sent with publish_to, so get the message from the pending acks map
            if let Some((_gt, Some(msg))) = self.pending_acks.get_mut(&id) {
                return self.tx.send_to_slim(Ok(msg.clone())).await;
            } else {
                return self.on_timer_failure(id);
            }
        }

        // this is a message sent to the group
        if let Some(message) = self.buffer.get(id as usize) {
            debug!("the message is still in the buffer, try to send it again to all the remotes");
            // get the names for which we are missing the acks
            // send the message to those destinations
            if let Some((gt, _)) = self.pending_acks.get(&id) {
                for n in &gt.missing_timers {
                    debug!(%id, dst = %n, "resend message");
                    let mut m = message.clone();
                    m.get_slim_header_mut().set_destination(n);

                    // send the message
                    self.tx.send_to_slim(Ok(m)).await?;
                }
            }
        } else {
            // the message is not in the buffer anymore so we can simply remove the timer
            debug!(
                %id,
                "message not in the buffer anymore, delete the associated timer",
            );
            return self.on_timer_failure(id);
        }

        Ok(())
    }

    pub fn on_failure(&mut self, id: u32, error: SessionError) -> Result<(), SessionError> {
        // remove all the state related to this timer
        if let Some((gt, _)) = self.pending_acks.get_mut(&id) {
            for n in &gt.missing_timers {
                if let Some(set) = self.pending_acks_per_endpoint.get_mut(n) {
                    set.remove(&id);
                }
            }
            gt.timer.stop();
        }

        self.pending_acks.remove(&id);

        // Signal failure to the ack notifier if present
        if let Some(tx) = self.ack_notifiers.remove(&id) {
            let _ = tx.send(Err(error));
        }

        Ok(())
    }

    pub fn on_timer_failure(&mut self, id: u32) -> Result<(), SessionError> {
        debug!(%id, "timer failure, clear state");
        self.on_failure(id, SessionError::MessageSendRetryFailed { id })
    }

    pub fn on_slim_failure(&mut self, error: SessionError) -> Result<(), SessionError> {
        let Some(session_ctx) = error.session_context() else {
            return Err(SessionError::UnexpectedError {
                source: Box::new(error),
            });
        };
        let message_id = session_ctx.message_id;
        debug!(%message_id, "slim reported failure, clear state");
        self.on_failure(message_id, error)
    }

    pub async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        // add endpoint to the list
        self.endpoints_list.insert(endpoint.clone());

        debug!(
            %endpoint,
            list_len = %self.endpoints_list.len(),
            "add endpoint",
        );

        // check if we need to flush the producer buffer
        if self.to_flush && self.endpoints_list.len() == 1 {
            self.to_flush = false;
            let messages: Vec<_> = self.buffer.iter().cloned().collect();
            for p in messages {
                let _ = self.set_timer_and_send(p).await;
            }
        }

        Ok(())
    }

    pub fn remove_endpoint(&mut self, endpoint: &Name) {
        debug!(
            %endpoint,
            list_len = %self.endpoints_list.len(),
            "remove endpoint",
        );
        // remove endpoint from the list and remove all the ack state
        // notice that no ack state may be associated to the endpoint
        // (e.g. endpoint added but no message sent)
        if let Some(set) = self.pending_acks_per_endpoint.get(endpoint) {
            for id in set {
                let mut delete = false;
                if let Some((gt, _)) = self.pending_acks.get_mut(id) {
                    debug!(
                        %id, %endpoint,
                        "try to remove timer",
                    );
                    gt.missing_timers.remove(endpoint);
                    // if no endpoint is left we remove the timer
                    if gt.missing_timers.is_empty() {
                        gt.timer.stop();
                        delete = true;
                    }
                }
                if delete {
                    debug!(%id, "no pending acks left, remove timer",);
                    self.pending_acks.remove(id);
                }
            }
        };

        debug!("remove endpoint name from everywhere");
        self.pending_acks_per_endpoint.remove(endpoint);
        self.endpoints_list.remove(endpoint);
        debug!(list_size = %self.endpoints_list.len(), "new list size");
    }

    pub fn start_drain(&mut self) {
        self.draining_state = SenderDrainStatus::Initiated;
        if self.pending_acks.is_empty() {
            self.draining_state = SenderDrainStatus::Completed;
        }
    }

    pub fn drain_completed(&self) -> bool {
        // Drain is complete if we're draining and no pending acks remain
        if self.draining_state == SenderDrainStatus::Completed
            || self.draining_state == SenderDrainStatus::Initiated && self.pending_acks.is_empty()
        {
            return true;
        }
        false
    }

    pub fn close(&mut self) {
        for (_, mut p) in self.pending_acks.drain() {
            p.0.timer.stop();
        }

        self.pending_acks.clear();
        self.pending_acks_per_endpoint.clear();

        // Notify all pending ack notifiers that the sender is closed
        for (_, tx) in self.ack_notifiers.drain() {
            let _ = tx.send(Err(SessionError::SessionClosed));
        }

        self.draining_state = SenderDrainStatus::Completed;
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
    async fn test_on_message_from_app() {
        // send messages from the app and verify that they arrive correctly formatted to SLIM
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_timeout() {
        // send message from the app and check for timeouts
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // Wait for first timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                sender
                    .on_timer_timeout(message_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for the retransmitted message
        let retransmission = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmitted message")
            .expect("channel closed")
            .expect("error message");

        // the message must be the same as the previous one
        assert_eq!(received, retransmission);

        // Wait for second timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                sender
                    .on_timer_timeout(message_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for the second retransmitted message
        let retransmission = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmitted message")
            .expect("channel closed")
            .expect("error message");

        // the message must be the same as the previous one
        assert_eq!(received, retransmission);

        // Wait for timer failure signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer failure signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerFailure { message_id, .. } => {
                sender
                    .on_timer_failure(message_id)
                    .expect("error handling timer failure");
            }
            _ => panic!("Expected TimerFailure signal, got: {:?}", signal),
        }

        // wait for timeout - should timeout since no more messages should be sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // no error should arrive to the application channel - errors are sent to ack_notifiers
        let res = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            res.is_err(),
            "Expected timeout (no app message) but got: {:?}",
            res
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_ack() {
        // send message from the app and get a timeout so no timer should be triggered
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // receive an ack from the remote
        let mut ack = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender
            .on_message(ack, None)
            .await
            .expect("error sending message");

        // wait for timeout - should timeout since no timer should trigger after the ack
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_ack_3_endpoints() {
        // send message from the app to 3 endpoints and get acks from all 3, so no timer should be triggered
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from all 3 remotes
        let mut ack1 = Message::builder()
            .source(remote1.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack2 = Message::builder()
            .source(remote2.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack2.get_session_header_mut().set_message_id(1);
        ack2.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::builder()
            .source(remote3.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send all 3 acks
        sender
            .on_message(ack1, None)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack2, None)
            .await
            .expect("error sending ack2");
        sender
            .on_message(ack3, None)
            .await
            .expect("error sending ack3");

        // wait for timeout - should timeout since no timer should trigger after all acks are received
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_3_endpoints() {
        // send message from the app to 3 endpoints but only get acks from 2, should trigger timers for the missing one
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::builder()
            .source(remote1.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::builder()
            .source(remote3.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender
            .on_message(ack1, None)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, None)
            .await
            .expect("error sending ack3");

        // Wait for first timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                sender
                    .on_timer_timeout(message_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // wait for timeout retransmission - should get a retransmission since remote2 didn't ack
        let retransmission = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the retransmission is the same message with same ID
        assert_eq!(
            retransmission.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(retransmission.get_id(), 1);
        // The destination should be set to remote2 (the one that didn't ack)
        assert_eq!(retransmission.get_dst(), remote2);

        // Wait for second timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                sender
                    .on_timer_timeout(message_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // wait for second timeout retransmission
        let retransmission2 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for second retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the second retransmission
        assert_eq!(
            retransmission2.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(retransmission2.get_id(), 1);
        assert_eq!(retransmission2.get_dst(), remote2);

        // Wait for timer failure signal (but don't handle it since this test focuses on retransmission)
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer failure signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerFailure { .. } => {
                // Just acknowledge the signal, don't call on_timer_failure as it would
                // try to send to the app channel which we're not monitoring in this test
            }
            _ => panic!("Expected TimerFailure signal, got: {:?}", signal),
        }

        // wait for timeout - should timeout since max retries (2) reached, no more messages should be sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_removing_endpoint() {
        // send message from the app to 3 endpoints but only get acks from 2, should trigger timers for the missing one
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::builder()
            .source(remote1.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::builder()
            .source(remote3.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender
            .on_message(ack1, None)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, None)
            .await
            .expect("error sending ack3");

        // remove endpoint 2
        sender.remove_endpoint(&remote2);

        // wait for timeout - this should expire as not timers should trigger
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_rtx_request() {
        // send message from the app to 3 endpoints but only get acks from 2,
        // then receive RTX request from endpoint 2. This should trigger retransmission and stop timers
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::builder()
            .source(remote1.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::builder()
            .source(remote3.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack from remote2)
        sender
            .on_message(ack1, None)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, None)
            .await
            .expect("error sending ack3");

        // Send RTX request from endpoint 2 instead of removing it
        let mut rtx_request = Message::builder()
            .source(remote2.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request.get_slim_header_mut().set_incoming_conn(Some(1));

        sender
            .on_message(rtx_request, None)
            .await
            .expect("error sending rtx_request");

        // Wait for the retransmission (RTX reply) to arrive at rx_slim
        let rtx_reply = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for rtx reply")
            .expect("channel closed")
            .expect("error in rtx reply");

        // Verify the RTX reply was sent correctly
        assert_eq!(
            rtx_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxReply
        );
        assert_eq!(rtx_reply.get_id(), 1);
        assert_eq!(rtx_reply.get_dst(), remote2);

        // wait for timeout - this should expire as timers should be stopped after RTX
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_request_and_reply_with_ack() {
        // send message with one endpoint, no ack received, timer triggers, then RTX request comes, reply sent, then ack received
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);

        // Wait for timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                sender
                    .on_timer_timeout(message_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // No ack sent, so wait for timeout retransmission
        let retransmission = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the retransmission
        assert_eq!(
            retransmission.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(retransmission.get_id(), 1);

        // Now simulate an RTX request from the remote
        let mut rtx_request = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request
            .get_slim_header_mut()
            .set_incoming_conn(Some(123)); // simulate an incoming connection

        // Send the RTX request
        sender
            .on_message(rtx_request, None)
            .await
            .expect("error sending rtx_request");

        // Wait for the RTX reply to arrive at rx_slim
        let rtx_reply = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for rtx reply")
            .expect("channel closed")
            .expect("error message");

        // Verify the RTX reply
        assert_eq!(
            rtx_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxReply
        );
        assert_eq!(rtx_reply.get_id(), 1);
        assert_eq!(rtx_reply.get_dst(), remote);
        assert_eq!(rtx_reply.get_slim_header().forward_to(), 123);

        // Now send an ack from the remote
        let mut ack = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender
            .on_message(ack, None)
            .await
            .expect("error sending ack");

        // wait for timeout - should timeout since timer should be stopped after ack
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_send_message_with_no_endpoints() {
        // send message without adding any endpoints, this should fail
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        // DO NOT add any endpoints - this is the key part of the test

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function - this should succeed but buffer the message
        let result = sender.on_message(message.clone(), None).await;

        // result should be ok
        assert!(result.is_ok(), "Expected Ok result, got: {:?}", result);

        // no message should arrive at rx_slim (message is buffered)
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected timeout (no message), but got: {:?}",
            res
        );

        // add the remote participant
        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // a message should arrive to rx_slim (buffered message is flushed)
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message was received with correct properties
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(received.get_id(), 1);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_multicast_session() {
        // Test sending a message with PUBLISH_TO metadata in a multicast session
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding participant");

        // Create a test message with PUBLISH_TO targeting remote2
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        // Send the message
        sender
            .on_message(message.clone(), None)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message properties
        assert_eq!(
            received.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        // For publish_to messages, fanout should be 1 (not MAX_FANOUT)
        assert_eq!(received.get_slim_header().get_fanout(), 1);
        assert_eq!(received.get_dst(), remote2);

        // Verify the message is NOT in the producer buffer (publish_to messages aren't buffered)
        assert!(sender.buffer.get(received.get_id() as usize).is_none());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_with_unknown_destination() {
        // Test sending a message with PUBLISH_TO to a destination not in the endpoints list
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let unknown_remote = Name::from_strings(["org", "ns", "unknown"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");

        // Create a message with PUBLISH_TO targeting an unknown endpoint
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(unknown_remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        // Send the message - should fail
        let result = sender.on_message(message, None).await;

        assert!(
            result.is_err_and(|e| { matches!(e, SessionError::UnknownDestination(_)) }),
            "Expected UnknownDestination error",
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_p2p_session_fallback() {
        // Test that PUBLISH_TO metadata is removed in point-to-point sessions
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a message with PUBLISH_TO metadata
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        // Send the message
        sender
            .on_message(message, None)
            .await
            .expect("error sending message");

        // Wait for the message
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Message should be treated as normal (sequential ID, buffered)
        assert_eq!(received.get_id(), 1); // Sequential ID
        assert!(sender.buffer.get(1).is_some()); // Should be buffered
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_ack_handling() {
        // Test that acks for PUBLISH_TO messages are handled correctly
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");

        // Create a PUBLISH_TO message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(message, None)
            .await
            .expect("error sending message");

        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        let message_id = received.get_id();

        // Send an ack with PUBLISH_TO metadata
        let mut ack = Message::builder()
            .source(remote2.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(message_id);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
        ack.metadata.insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(ack, None)
            .await
            .expect("error sending ack");

        // Verify the pending_acks is cleared after ack is received
        assert!(!sender.pending_acks.contains_key(&message_id));

        // Wait for timeout - no timers should trigger
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_timeout_retransmission() {
        // Test timeout and retransmission for PUBLISH_TO messages
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");

        // Create a PUBLISH_TO message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(message, None)
            .await
            .expect("error sending message");

        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        let message_id = received.get_id();

        // Wait for timeout signal
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout {
                message_id: timer_id,
                ..
            } => {
                assert_eq!(timer_id, message_id);
                sender
                    .on_timer_timeout(timer_id)
                    .await
                    .expect("error handling timeout");
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for retransmission
        let retransmission = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(retransmission.get_id(), message_id);
        assert_eq!(retransmission.get_dst(), remote2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_endpoint_removed_before_timeout() {
        // Test that PUBLISH_TO timer is cleaned up when endpoint is removed
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");

        // Create a PUBLISH_TO message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(message, None)
            .await
            .expect("error sending message");

        let _received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Remove the endpoint before timeout
        sender.remove_endpoint(&remote2);

        // No timeout signal should arrive since the timer was cleaned up when endpoint was removed
        // Wait to ensure no signal arrives
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected no timer signal but got: {:?}", res);

        // No retransmission should occur since endpoint was removed
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // Verify pending_acks is cleaned up (since the target endpoint was removed)
        assert!(sender.pending_acks.is_empty());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ack_notifiers_with_publish_to() {
        // Test that ack notifiers work correctly with PUBLISH_TO messages
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Create a PUBLISH_TO message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        // Create oneshot channel for ack notification
        let (ack_tx, ack_rx) = oneshot::channel();

        sender
            .on_message(message, Some(ack_tx))
            .await
            .expect("error sending message");

        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        let message_id = received.get_id();

        // Send an ack
        let mut ack = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(message_id);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
        ack.metadata.insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(ack, None)
            .await
            .expect("error sending ack");

        // Verify ack notifier receives success
        let ack_result = timeout(Duration::from_millis(100), ack_rx)
            .await
            .expect("timeout waiting for ack notification")
            .expect("ack notification channel closed");

        assert!(ack_result.is_ok());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_mixed_normal_and_publish_to_messages() {
        // Test sending both normal and PUBLISH_TO messages in the same session
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            false,
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding participant");

        let source = Name::from_strings(["org", "ns", "source"]);

        // Send a normal multicast message
        let mut normal_msg = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("normal", vec![1])
            .build_publish()
            .unwrap();
        normal_msg.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        sender
            .on_message(normal_msg, None)
            .await
            .expect("error sending normal message");

        let received_normal = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(received_normal.get_id(), 1); // Sequential ID
        assert_eq!(
            received_normal.get_slim_header().get_fanout(),
            SessionSender::MAX_FANOUT
        );

        // Send a PUBLISH_TO message
        let mut publish_to_msg = Message::builder()
            .source(source.clone())
            .destination(remote1.clone())
            .application_payload("publish_to", vec![2])
            .build_publish()
            .unwrap();
        publish_to_msg.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        publish_to_msg
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(publish_to_msg, None)
            .await
            .expect("error sending publish_to message");

        let received_publish_to = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // PUBLISH_TO should have random ID (not sequential)
        assert_ne!(received_publish_to.get_id(), 2);
        assert_eq!(received_publish_to.get_slim_header().get_fanout(), 1);
        assert_eq!(received_publish_to.get_dst(), remote1);

        // Verify normal message is in buffer but PUBLISH_TO is not
        assert!(sender.buffer.get(1).is_some());
        assert!(
            sender
                .buffer
                .get(received_publish_to.get_id() as usize)
                .is_none()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_send_sequential_messages() {
        // Test: Send messages 1, 2, 3 sequentially with shutdown_send=true.
        // Verify no messages reach SLIM, ack notifiers receive errors.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            true, // shutdown_send enabled
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        let source = Name::from_strings(["org", "ns", "source"]);

        // Send messages 1, 2, 3 sequentially with ack notifiers
        for msg_id in 1..=3 {
            let mut message = Message::builder()
                .source(source.clone())
                .destination(remote.clone())
                .application_payload(
                    &format!("test_payload_{}", msg_id),
                    vec![msg_id as u8, (msg_id + 1) as u8],
                )
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

            // Create oneshot channel for ack notification
            let (ack_tx, ack_rx) = oneshot::channel();

            let result = sender.on_message(message, Some(ack_tx)).await;

            // Verify on_message returns error when shutdown_send is true
            assert!(
                result.is_err(),
                "Expected error from on_message but got: {:?}",
                result
            );
            assert!(
                matches!(result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error"
            );

            // Verify ack notifier receives error
            let ack_result = timeout(Duration::from_millis(100), ack_rx)
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for ack notification {}", msg_id))
                .unwrap_or_else(|_| panic!("channel closed for message {}", msg_id));

            assert!(
                ack_result.is_err(),
                "Expected error result but got: {:?}",
                ack_result
            );
            assert!(
                matches!(ack_result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error in ack notifier"
            );
        }

        // Verify no messages were sent to SLIM
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to SLIM but got: {:?}",
            res
        );

        // Verify no timers were created (pending_acks should be empty)
        assert!(sender.pending_acks.is_empty(), "Expected no pending acks");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_send_multiple_endpoints() {
        // Test: Add multiple endpoints and send messages with shutdown_send=true.
        // Verify no messages reach SLIM and an error is returned.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
            true, // shutdown_send enabled
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(&remote1)
            .await
            .expect("error adding remote1");
        sender
            .add_endpoint(&remote2)
            .await
            .expect("error adding remote2");
        sender
            .add_endpoint(&remote3)
            .await
            .expect("error adding remote3");

        let source = Name::from_strings(["org", "ns", "source"]);

        // Send 3 messages to the group with ack notifiers
        for msg_num in 1..=3 {
            let mut message = Message::builder()
                .source(source.clone())
                .destination(group.clone())
                .application_payload(&format!("test_payload_{}", msg_num), vec![msg_num as u8])
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

            let (ack_tx, ack_rx) = oneshot::channel();

            let result = sender.on_message(message, Some(ack_tx)).await;

            // Verify on_message returns error when shutdown_send is true
            assert!(
                result.is_err(),
                "Expected error from on_message but got: {:?}",
                result
            );
            assert!(
                matches!(result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error"
            );

            // Verify ack notifier receives error
            let ack_result = timeout(Duration::from_millis(100), ack_rx)
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for ack {}", msg_num))
                .unwrap_or_else(|_| panic!("channel closed for message {}", msg_num));

            assert!(
                ack_result.is_err(),
                "Expected error result but got: {:?}",
                ack_result
            );
            assert!(
                matches!(ack_result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error in ack notifier"
            );
        }

        // Verify no messages were sent to SLIM
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to SLIM but got: {:?}",
            res
        );

        // Verify no timers were created
        assert!(sender.pending_acks.is_empty(), "Expected no pending acks");
        assert!(
            sender.pending_acks_per_endpoint.is_empty(),
            "Expected no pending acks per endpoint"
        );

        // Verify buffer is empty (messages not buffered when shutdown_send is true)
        assert!(sender.buffer.get(1).is_none(), "Expected empty buffer");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_send_with_rtx_request() {
        // Test: With shutdown_send=true, verify RTX requests get error responses
        // since messages are not buffered.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            true, // shutdown_send enabled
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);
        let source = Name::from_strings(["org", "ns", "source"]);

        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Send a message (will be dropped due to shutdown_send)
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        let result = sender.on_message(message, None).await;
        assert!(
            result.is_err(),
            "Expected error from on_message but got: {:?}",
            result
        );
        assert!(
            matches!(result, Err(SessionError::SessionSenderShutdown)),
            "Expected SessionSenderShutdown error"
        );

        // Verify no message was sent to SLIM
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no message to SLIM but got: {:?}",
            res
        );

        // Now send an RTX request for message ID 1
        let mut rtx_request = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request
            .get_slim_header_mut()
            .set_incoming_conn(Some(123));

        sender
            .on_message(rtx_request, None)
            .await
            .expect("error sending rtx request");

        // Wait for RTX reply with error
        let rtx_reply = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for rtx reply")
            .expect("channel closed")
            .expect("error in rtx reply");

        // Verify the RTX reply has an error (message not in buffer)
        assert_eq!(
            rtx_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxReply
        );
        assert_eq!(rtx_reply.get_id(), 1);
        assert_eq!(rtx_reply.get_dst(), remote);
        assert!(
            rtx_reply.get_error().is_some(),
            "Expected error in RTX reply"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_send_with_buffer_flush() {
        // Test: Send messages with shutdown_send=true BEFORE adding endpoints,
        // then add endpoint. Verify messages are NOT flushed since shutdown_send prevents transmission.
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
            true, // shutdown_send enabled
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);
        let source = Name::from_strings(["org", "ns", "source"]);

        // Send 3 messages WITHOUT adding endpoint first (normally would be buffered for flush)
        for msg_id in 1..=3 {
            let mut message = Message::builder()
                .source(source.clone())
                .destination(remote.clone())
                .application_payload(
                    &format!("test_payload_{}", msg_id),
                    vec![msg_id as u8, (msg_id + 1) as u8],
                )
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

            let (ack_tx, ack_rx) = oneshot::channel();

            let result = sender.on_message(message, Some(ack_tx)).await;

            // Verify on_message returns error when shutdown_send is true
            assert!(
                result.is_err(),
                "Expected error from on_message but got: {:?}",
                result
            );
            assert!(
                matches!(result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error"
            );

            // Verify ack notifier receives error
            let ack_result = timeout(Duration::from_millis(100), ack_rx)
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for ack {}", msg_id))
                .unwrap_or_else(|_| panic!("channel closed for message {}", msg_id));

            assert!(
                ack_result.is_err(),
                "Expected error result but got: {:?}",
                ack_result
            );
            assert!(
                matches!(ack_result, Err(SessionError::SessionSenderShutdown)),
                "Expected SessionSenderShutdown error in ack notifier"
            );
        }

        // Verify no messages were sent to SLIM yet
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages to SLIM but got: {:?}",
            res
        );

        // Now add the endpoint (normally would trigger flush)
        sender
            .add_endpoint(&remote)
            .await
            .expect("error adding participant");

        // Verify messages are still NOT sent to SLIM (shutdown_send prevents flush)
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no messages after endpoint add but got: {:?}",
            res
        );

        // Verify buffer is empty (messages not buffered when shutdown_send is true)
        assert!(sender.buffer.get(1).is_none(), "Expected empty buffer");
        assert!(!sender.to_flush, "Expected to_flush flag to be false");
    }
}
