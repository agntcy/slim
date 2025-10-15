// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use slim_datapath::api::{SessionHeader, SlimHeader};
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_datapath::{api::ProtoMessage as Message, messages::Name};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error};

use crate::MessageDirection;
use crate::{
    SessionError, Transmitter,
    common::SessionMessage,
    producer_buffer::ProducerBuffer,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
};

const DRAIN_TIMER_ID: u32 = 0;

#[allow(dead_code)]
struct GroupTimer {
    /// list of names for which we did not get an ack yet
    missing_timers: HashSet<Name>,

    /// the timer
    timer: Timer,
}

#[allow(dead_code)]
struct SessionSenderInternal<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// buffer storing messages coming from the application
    buffer: ProducerBuffer,

    /// timer factory to crate timers for acks
    timer_factory: TimerFactory,

    /// list of pending acks for each message
    pending_acks: HashMap<u32, GroupTimer>,

    /// list of timer ids associated for each endpoint
    pending_acks_per_endpoint: HashMap<Name, HashSet<u32>>,

    /// list of the endpoints in the conversation
    /// on message send, create an ack timer for each endpoint in the list
    /// on endpoint remove, delete all the acks to the endpoint
    endpoints_list: HashSet<Name>,

    /// message id, used if the session is sequential
    next_id: u32,

    /// session id where to send the messages
    session_id: u32,

    /// send packets to slim or the app
    tx: T,

    /// received for signals
    rx: Receiver<SessionMessage>,

    /// if true the sender is reliable and handles acks and rtx
    is_reliable: bool,

    /// drain state - when true, no new messages from app are accepted
    is_draining: bool,

    /// drain timer - when set, we're waiting for pending acks during grace period
    drain_timer: Option<Timer>,
}

#[allow(dead_code)]
impl<T> SessionSenderInternal<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn new(
        timer_settings: TimerSettings,
        session_id: u32,
        is_reliable: bool,
        tx: T,
        tx_signals: Sender<SessionMessage>,
        rx_signals: Receiver<SessionMessage>,
    ) -> Self {
        let factory = TimerFactory::new(timer_settings, tx_signals);

        SessionSenderInternal {
            buffer: ProducerBuffer::with_capacity(512),
            timer_factory: factory,
            pending_acks: HashMap::new(),
            pending_acks_per_endpoint: HashMap::new(),
            endpoints_list: HashSet::new(),
            next_id: 0,
            session_id,
            tx,
            rx: rx_signals,
            is_reliable,
            is_draining: false,
            drain_timer: None,
        }
    }

    async fn on_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::P2PReliable => {
                debug!("received P2P reliable message");
                self.on_publish_message(message).await?
            }
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg => {
                debug!("received multicast message");
                self.on_publish_message(message).await?
            }
            slim_datapath::api::ProtoSessionMessageType::P2PAck => {
                debug!("received ack message");
                if !self.is_reliable {
                    return Ok(());
                }
                self.on_ack_message(&message);
            }
            slim_datapath::api::ProtoSessionMessageType::RtxRequest => {
                debug!("received rtx message");
                if !self.is_reliable {
                    return Ok(());
                }
                // receiving an rtx request for a message we stop the
                // corresponding ack timer. For this point on if the
                // message is not delivered the receiver side will keep
                // asking for it
                self.on_ack_message(&message);
                // after the ack removal, process the rtx request
                self.on_rtx_message(message).await?
            }
            _ => {
                // TODO: Add missing message types (e.g. Channel messages)
                debug!("unexpected message type");
            }
        }
        Ok(())
    }

    async fn on_publish_message(&mut self, mut message: Message) -> Result<(), SessionError> {
        if self.is_draining {
            return Err(SessionError::Processing(
                "sender is draining, cannot send new messages".to_string(),
            ));
        }

        if self.endpoints_list.is_empty() {
            return Err(SessionError::Processing(
                "endpoint list is empty, cannot create timers".to_string(),
            ));
        }

        // compute message id
        // by increasing next_id before assign it to message_id
        // we always skip message 0 (used as drain timer id)
        self.next_id += 1;
        let message_id = self.next_id;

        debug!("send new message with id {}", message_id);

        // Get a mutable reference to the message header
        let header = message.get_session_header_mut();

        // Set the session id and message id
        header.set_message_id(message_id);
        header.set_session_id(self.session_id);

        if self.is_reliable {
            debug!("reliable sender, set all timers");
            // add the message to the producer buffer. this is used to re-send
            // messages when acks are missing and to handle retrasmissions
            self.buffer.push(message.clone());

            // create a timer for the new packet and update the state
            let gt = GroupTimer {
                missing_timers: self.endpoints_list.clone(),
                timer: self.timer_factory.create_and_start_timer(message_id, None),
            };

            // insert in pending acks
            self.pending_acks.insert(message_id, gt);

            // insert in pending acks per endpoint
            for n in &self.endpoints_list {
                debug!("add timer for message {} for remote {}", message_id, n);
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
        self.tx
            .send_to_slim(Ok(message))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    fn on_ack_message(&mut self, message: &Message) {
        let source = message.get_source();
        let message_id = message.get_id();
        debug!("received ack message for id {} from {}", message_id, source);

        let mut delete = false;
        // remove the source from the pending acks
        // notice that the state for this ack id may not exist if all the endpoints
        // associated to it where removed before getting the acks
        if let Some(gt) = self.pending_acks.get_mut(&message_id) {
            debug!("try to remove {} from pending acks", source);
            gt.missing_timers.remove(&source);
            if gt.missing_timers.is_empty() {
                debug!("all acks received, remove timer");
                // all acks received. stop the timer and remove the entry
                gt.timer.stop();
                delete = true;
            }
        }

        // remove the pending ack from the name
        // also here the endpoint may not exists anymore
        if let Some(set) = self.pending_acks_per_endpoint.get_mut(&source) {
            debug!(
                "remove message id {} from pendings acks for {}",
                message_id, source
            );
            // here we do not remove the name even if the set is empty
            // remove it only if endpoint is deleted
            set.remove(&message_id);
        }

        // all acks received for this timer, remove it
        if delete {
            debug!(
                "all acks received for message id {}, remove timer",
                message_id
            );
            self.pending_acks.remove(&message_id);
        }
    }

    async fn on_rtx_message(&mut self, message: Message) -> Result<(), SessionError> {
        let source = message.get_source();
        let message_id = message.get_id();
        let incoming_conn = message.get_incoming_conn();

        debug!(
            "received rtx request for message {} from remote {}",
            message_id, source
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
            msg.get_session_header_mut().set_destination(&source);

            println!("msg: {:?}", msg);
            // send the message
            self.tx
                .send_to_slim(Ok(msg))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()))
        } else {
            debug!("the message does not exists anymore, send and error");
            let destination = message.get_dst();
            // the message is not in the buffer send an RTX with error
            let flags = SlimHeaderFlags::default()
                .with_forward_to(incoming_conn)
                .with_error(true);

            let slim_header = Some(SlimHeader::new(&destination, &source, Some(flags)));

            // no need to set source and destiona here
            let session_header = Some(SessionHeader::new(
                message.get_session_type().into(),
                slim_datapath::api::ProtoSessionMessageType::RtxReply.into(),
                self.session_id,
                message_id,
                &None,
                &None,
            ));

            let msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

            // send the message
            self.tx
                .send_to_slim(Ok(msg))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()))
        }
    }

    async fn on_timer_timeout(&mut self, id: u32) -> Result<(), SessionError> {
        debug!("timeout for message {}", id);
        if let Some(message) = self.buffer.get(id as usize) {
            debug!("the message is still in the buffer, try to send it again to all the remotes");
            // get the names for which we are missing the acks
            // send the message to those destinations
            if let Some(gt) = self.pending_acks.get(&id) {
                for n in &gt.missing_timers {
                    debug!("resend message {} to {}", id, n);
                    let mut m = message.clone();
                    m.get_slim_header_mut().set_destination(n);
                    m.get_session_header_mut().set_destination(n);

                    // send the message
                    self.tx
                        .send_to_slim(Ok(m))
                        .await
                        .map_err(|e| SessionError::SlimTransmission(e.to_string()))?;
                }
            }
        } else {
            // the message is not in the buffer anymore so we can simply remove the timer
            debug!(
                "the message {} is not in the buffer anymore, delete the associated timer",
                id
            );
            return self.on_timer_failure(id).await;
        }
        Ok(())
    }

    async fn on_timer_failure(&mut self, id: u32) -> Result<(), SessionError> {
        debug!("timer failure for message id {}, clear state", id);
        // remove all the state related to this timer
        if let Some(gt) = self.pending_acks.get_mut(&id) {
            for n in &gt.missing_timers {
                if let Some(set) = self.pending_acks_per_endpoint.get_mut(n) {
                    set.remove(&id);
                }
            }
            gt.timer.stop();
        }

        self.pending_acks.remove(&id);

        // notify the application that the message was not delivered correctly
        self.tx
            .send_to_app(Err(SessionError::Processing(format!(
                "error send message {}. stop retrying",
                id
            ))))
            .await
    }

    fn add_endpoint(&mut self, endpoint: Name) {
        debug!("add endpoint {}", endpoint);
        // add endpoint to the list
        self.endpoints_list.insert(endpoint);
    }

    fn rm_endpoint(&mut self, endpoint: &Name) {
        debug!("remove endpoint {}", endpoint);
        // remove endpoint from the list and remove all the ack state
        // notice that no ack state may be associated to the endpoint
        // (e.g. endpoint added but no message sent)
        if let Some(set) = self.pending_acks_per_endpoint.get(endpoint) {
            for id in set {
                let mut delete = false;
                if let Some(gt) = self.pending_acks.get_mut(id) {
                    debug!(
                        "try to remove timer {} for removed endpoint {}",
                        id, endpoint
                    );
                    gt.missing_timers.remove(endpoint);
                    // if no endpoint is left we remove the timer
                    if gt.missing_timers.is_empty() {
                        gt.timer.stop();
                        delete = true;
                    }
                }
                if delete {
                    debug!("no pending acks left, remove timer {}", id);
                    self.pending_acks.remove(id);
                }
            }
        };

        debug!("remove endpoint name from everywhere");
        self.pending_acks_per_endpoint.remove(endpoint);
        self.endpoints_list.remove(endpoint);
    }

    fn start_drain(&mut self, grace_period_ms: u64) {
        debug!("starting drain with grace period {}ms", grace_period_ms);
        self.is_draining = true;

        // If we have pending acks, start a grace period timer
        if !self.pending_acks.is_empty() {
            debug!("pending acks exist, starting grace period timer");
            // Use a special timer ID for drain
            let drain_timer_id = DRAIN_TIMER_ID;
            let timer = Timer::new(
                drain_timer_id,
                crate::timer::TimerType::Constant,
                std::time::Duration::from_millis(grace_period_ms),
                None,
                Some(1), // max_retries - only fire once for drain
            );
            self.timer_factory.start_timer(&timer, None);
            self.drain_timer = Some(timer);
        } else {
            debug!("no pending acks, can stop immediately");
            // No pending acks, we can stop immediately
            self.drain_timer = None;
        }
    }

    fn check_drain_completion(&self) -> bool {
        // Drain is complete if we're draining and no pending acks remain
        self.is_draining && self.pending_acks.is_empty()
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                next = self.rx.recv() => {
                    match next {
                        Some(SessionMessage::OnMessage { message, direction: _ }) => {
                            debug!("received OnMessage");
                            if self.on_message(message).await.is_err() {
                                error!("error sending message");
                            }
                            // Check if drain is complete after processing messages (especially acks)
                            if self.check_drain_completion() {
                                debug!("drain completed, all acks received");
                                break;
                            }
                        }
                        Some(SessionMessage::TimerTimeout { message_id, timeouts: _ , name: _}) => {
                            debug!("timer {} timeout", message_id);
                            // Check if this is the drain timer
                            if message_id == DRAIN_TIMER_ID {
                                debug!("drain grace period expired, stopping sender");
                                break; // Exit the loop to stop the sender
                            } else if self.on_timer_timeout(message_id).await.is_err() {
                                error!("error processing message timeout");
                            }
                        },
                        Some(SessionMessage::TimerFailure { message_id, timeouts: _ , name: _}) => {
                            debug!("timer {} failed", message_id);
                            if self.on_timer_failure(message_id).await.is_err() {
                                error!("error processing timer failure");
                            }
                        },
                        Some(SessionMessage::AddEndpoint {endpoint}) => {
                            debug!("add new endpoint {}", endpoint);
                            self.add_endpoint(endpoint);
                        }
                        Some(SessionMessage::RemoveEndpoint {endpoint}) => {
                            debug!("remove endpoint {}", endpoint);
                            self.rm_endpoint(&endpoint);
                        }
                        Some(SessionMessage::Drain { grace_period_ms }) => {
                            debug!("received drain signal with grace period {}ms", grace_period_ms);
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
                            debug!("stop sender on session {}", self.session_id);
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) struct SessionSender<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    tx: Sender<SessionMessage>,
    // if true do not send signals
    drain: bool,
    _phantom: PhantomData<T>,
}

#[allow(dead_code)]
impl<T> SessionSender<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        timer_settings: TimerSettings,
        session_id: u32,
        is_reliable: bool,
        tx_transmitter: T,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        let internal_sender = SessionSenderInternal::new(
            timer_settings,
            session_id,
            is_reliable,
            tx_transmitter,
            tx.clone(),
            rx,
        );

        // Spawn the processing loop
        tokio::spawn(async move {
            internal_sender.process_loop().await;
        });
        SessionSender {
            tx,
            drain: false,
            _phantom: PhantomData,
        }
    }

    /// To be used to send a message to slim,
    /// handle an ack or an rtx request
    pub async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // allow only acks and RTX requests
        if self.drain {
            match message.get_session_message_type() {
                slim_datapath::api::ProtoSessionMessageType::P2PAck
                | slim_datapath::api::ProtoSessionMessageType::RtxRequest => {
                    // Allow acks and RTX requests
                }
                _ => {
                    return Err(SessionError::Processing("sender is closing".to_string()));
                }
            }
        }
        self.tx
            .send(SessionMessage::OnMessage { message, direction })
            .await
            .map_err(|e| SessionError::Processing(format!("Failed to send message: {}", e)))
    }

    /// Add a endpoint to the sender
    pub async fn add_endpoint(&self, endpoint: Name) -> Result<(), SessionError> {
        if self.drain {
            return Err(SessionError::Processing("sender is closing".to_string()));
        }
        self.tx
            .send(SessionMessage::AddEndpoint { endpoint })
            .await
            .map_err(|e| SessionError::Processing(format!("Failed to add endpoint: {}", e)))
    }

    /// Remove a endpoint from the sender
    pub async fn remove_endpoint(&self, endpoint: Name) -> Result<(), SessionError> {
        if self.drain {
            return Err(SessionError::Processing("sender is closing".to_string()));
        }
        self.tx
            .send(SessionMessage::RemoveEndpoint { endpoint })
            .await
            .map_err(|e| SessionError::Processing(format!("Failed to remove endpoint: {}", e)))
    }

    /// Initiate graceful shutdown - no new messages accepted, wait for pending acks
    /// during grace period before forcefully stopping
    pub async fn drain(&mut self, grace_period_ms: u64) -> Result<(), SessionError> {
        if self.drain {
            return Err(SessionError::Processing("sender is closing".to_string()));
        }
        self.drain = true;

        self.tx
            .send(SessionMessage::Drain { grace_period_ms })
            .await
            .map_err(|e| SessionError::Processing(format!("Failed to initiate drain: {}", e)))
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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(remote.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 1);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_timeout() {
        // send message from the app and check for timeouts
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(remote.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 1);

        // wait for timeout
        let retransmission = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // the message must be the same as the previous one
        assert_eq!(received, retransmission);

        // wait for timeout
        let retransmission = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // the message must be the same as the previous one
        assert_eq!(received, retransmission);

        // wait for timeout - should timeout since no more messages should be sent
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // an error should arrive to the application
        let res = timeout(Duration::from_millis(800), rx_app.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed");

        // Check that we received an error as expected
        match res {
            Err(SessionError::Processing(msg)) => {
                assert!(
                    msg.contains("error send message 1. stop retrying"),
                    "Expected timer failure error message, got: {}",
                    msg
                );
            }
            _ => panic!(
                "Expected SessionError::Processing with timer failure message, got: {:?}",
                res
            ),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_ack() {
        // send message from the app and get a timeout so no timer should be triggered
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(remote.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 1);

        // receive an ack from the remote
        let mut ack = Message::new_publish(&remote, &source, None, "", vec![]);

        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        sender
            .on_message(ack, MessageDirection::North)
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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(remote1.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote2.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote3.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &group, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from all 3 remotes
        let mut ack1 = Message::new_publish(&remote1, &source, None, "", vec![]);
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        let mut ack2 = Message::new_publish(&remote2, &source, None, "", vec![]);
        ack2.get_session_header_mut().set_message_id(1);
        ack2.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        let mut ack3 = Message::new_publish(&remote3, &source, None, "", vec![]);
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        // Send all 3 acks
        sender
            .on_message(ack1, MessageDirection::North)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack2, MessageDirection::North)
            .await
            .expect("error sending ack2");
        sender
            .on_message(ack3, MessageDirection::North)
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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(remote1.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote2.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote3.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &group, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MulticastMsg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::new_publish(&remote1, &source, None, "", vec![]);
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        let mut ack3 = Message::new_publish(&remote3, &source, None, "", vec![]);
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender
            .on_message(ack1, MessageDirection::North)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, MessageDirection::North)
            .await
            .expect("error sending ack3");

        // wait for timeout retransmission - should get a retransmission since remote2 didn't ack
        let retransmission = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the retransmission is the same message with same ID
        assert_eq!(
            retransmission.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(retransmission.get_id(), 1);
        // The destination should be set to remote2 (the one that didn't ack)
        assert_eq!(retransmission.get_dst(), remote2);

        // wait for second timeout retransmission
        let retransmission2 = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for second retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the second retransmission
        assert_eq!(
            retransmission2.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(retransmission2.get_id(), 1);
        assert_eq!(retransmission2.get_dst(), remote2);

        // wait for timeout - should timeout since max retries (2) reached, no more messages should be sent
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_removing_endpoint() {
        // send message from the app to 3 endpoints but only get acks from 2, should trigger timers for the missing one
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(remote1.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote2.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote3.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &group, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MulticastMsg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::new_publish(&remote1, &source, None, "", vec![]);
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        let mut ack3 = Message::new_publish(&remote3, &source, None, "", vec![]);
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender
            .on_message(ack1, MessageDirection::North)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, MessageDirection::North)
            .await
            .expect("error sending ack3");

        // remove endpoint 2
        sender
            .remove_endpoint(remote2)
            .await
            .expect("error removing remote2");

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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender
            .add_endpoint(remote1.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote2.clone())
            .await
            .expect("error adding endpoint");
        sender
            .add_endpoint(remote3.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &group, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to MulticastMsg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MulticastMsg);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(received.get_id(), 1);

        // receive acks from only remote1 and remote3 (missing ack from remote2)
        let mut ack1 = Message::new_publish(&remote1, &source, None, "", vec![]);
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        let mut ack3 = Message::new_publish(&remote3, &source, None, "", vec![]);
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        // Send only 2 out of 3 acks (missing ack from remote2)
        sender
            .on_message(ack1, MessageDirection::North)
            .await
            .expect("error sending ack1");
        sender
            .on_message(ack3, MessageDirection::North)
            .await
            .expect("error sending ack3");

        // Send RTX request from endpoint 2 instead of removing it
        let mut rtx_request = Message::new_publish(&remote2, &source, None, "", vec![]);
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request.get_slim_header_mut().set_incoming_conn(Some(1));

        sender
            .on_message(rtx_request, MessageDirection::North)
            .await
            .expect("error sending rtx request");

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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(remote.clone())
            .await
            .expect("error adding endpoint");

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function
        sender
            .on_message(message.clone(), MessageDirection::South)
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
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(received.get_id(), 1);

        // No ack sent, so wait for timeout retransmission
        let retransmission = timeout(Duration::from_millis(800), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        // Verify the retransmission
        assert_eq!(
            retransmission.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::P2PReliable
        );
        assert_eq!(retransmission.get_id(), 1);

        // Now simulate an RTX request from the remote
        let mut rtx_request = Message::new_publish(&remote, &source, None, "", vec![]);
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request
            .get_slim_header_mut()
            .set_incoming_conn(Some(123)); // simulate an incoming connection

        // Send the RTX request
        sender
            .on_message(rtx_request, MessageDirection::North)
            .await
            .expect("error sending rtx request");

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
        let mut ack = Message::new_publish(&remote, &source, None, "", vec![]);
        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        sender
            .on_message(ack, MessageDirection::North)
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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        // DO NOT add any endpoints - this is the key part of the test

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);

        // Set session message type to P2PReliable for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        // Send the message using on_message function - this should fail
        let result = sender
            .on_message(message.clone(), MessageDirection::South)
            .await;

        // The on_message call should succeed (it just queues the message), but the internal processing should fail
        // We need to check that no message was actually sent to rx_slim
        assert!(
            result.is_ok(),
            "on_message should succeed even with no endpoints"
        );

        // Wait a bit and verify no message was sent
        let res = timeout(Duration::from_millis(200), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no message to be sent when no endpoints are added, but got: {:?}",
            res
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_drain_with_pending_acks() {
        // Test drain functionality - send message, initiate drain, then send ack
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(settings, 10, true, tx);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender
            .add_endpoint(remote.clone())
            .await
            .expect("error adding endpoint");

        // Create and send a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message =
            Message::new_publish(&source, &remote, None, "test_payload", vec![1, 2, 3, 4]);
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        sender
            .on_message(message.clone(), MessageDirection::South)
            .await
            .expect("error sending message");

        // Wait for the message
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(received.get_id(), 1);

        // Initiate drain with 2 second grace period
        sender.drain(2000).await.expect("error initiating drain");

        // Try to send another message - this should be rejected
        let mut message2 =
            Message::new_publish(&source, &remote, None, "test_payload2", vec![5, 6, 7, 8]);
        message2.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PReliable);

        let result = sender.on_message(message2, MessageDirection::South).await;

        // The message should be rejected because we're draining
        assert!(
            result.is_err(),
            "Message should be rejected when sender is draining"
        );
        match result {
            Err(SessionError::Processing(msg)) => {
                assert!(
                    msg.contains("sender is closing"),
                    "Expected 'sender is closing' error, got: {}",
                    msg
                );
            }
            _ => panic!(
                "Expected SessionError::Processing with 'sender is closing' message, got: {:?}",
                result
            ),
        }

        // No second message should be sent to SLIM since we're draining
        let res = timeout(Duration::from_millis(200), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected timeout since no new message should be sent during drain"
        );

        // Send ack for the first message
        let mut ack = Message::new_publish(&remote, &source, None, "", vec![]);
        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::P2PAck);

        sender
            .on_message(ack, MessageDirection::North)
            .await
            .expect("error sending ack");

        // Give some time for the sender to process the ack and complete drain
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_drain_with_no_pending_acks() {
        // Test drain when no pending acks exist - should stop immediately
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, _) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(settings, 10, true, tx);

        // Initiate drain without any pending messages
        let result = sender.drain(1000).await;
        assert!(
            result.is_ok(),
            "Drain should succeed even with no pending acks"
        );

        // Try to add an endpoint after drain - this should fail
        let remote = Name::from_strings(["org", "ns", "remote"]);
        let add_result = sender.add_endpoint(remote.clone()).await;
        assert!(
            add_result.is_err(),
            "Adding endpoint should fail when sender is draining"
        );
        match add_result {
            Err(SessionError::Processing(msg)) => {
                assert!(
                    msg.contains("sender is closing"),
                    "Expected 'sender is closing' error, got: {}",
                    msg
                );
            }
            _ => panic!(
                "Expected SessionError::Processing with 'sender is closing' message, got: {:?}",
                add_result
            ),
        }
    }
}
