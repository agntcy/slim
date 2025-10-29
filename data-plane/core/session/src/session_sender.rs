// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use slim_datapath::api::ProtoSessionType;
use slim_datapath::{api::ProtoMessage as Message, messages::Name};
use tokio::sync::mpsc::Sender;
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
    pending_acks: HashMap<u32, GroupTimer>,

    /// list of timer ids associated for each endpoint
    pending_acks_per_endpoint: HashMap<Name, HashSet<u32>>,

    /// list of the endpoints in the conversation
    /// on message send, create an ack timer for each endpoint in the list
    /// on endpoint remove, delete all the acks to the endpoint
    endpoints_list: HashSet<Name>,

    /// message id, used if the session is sequential
    next_id: u32,

    /// session id to had in the message header
    session_id: u32,

    /// session type to set in the message header
    session_type: ProtoSessionType,

    /// send packets to slim or the app
    tx: SessionTransmitter,

    /// drain state - when true, no new messages from app are accepted
    draining_state: SenderDrainStatus,
}

#[allow(dead_code)]
impl SessionSender {
    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        session_type: ProtoSessionType,
        tx: SessionTransmitter,
        tx_signals: Option<Sender<SessionMessage>>,
    ) -> Self {
        let factory = if let Some(settings) = timer_settings
            && let Some(tx) = tx_signals
        {
            Some(TimerFactory::new(settings, tx))
        } else {
            None
        };

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
            draining_state: SenderDrainStatus::NotDraining,
        }
    }

    pub async fn on_message(&mut self, message: Message) -> Result<(), SessionError> {
        if self.draining_state == SenderDrainStatus::Completed {
            return Err(SessionError::Processing(
                "sender closed, drop message".to_string(),
            ));
        }

        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::Msg => {
                debug!("received message");
                if self.draining_state == SenderDrainStatus::Initiated {
                    // draining period is started, do no accept any new message
                    return Err(SessionError::Processing(
                        "drain started do no accept new messages".to_string(),
                    ));
                }
                self.on_publish_message(message).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::MsgAck => {
                debug!("received ack message");
                if self.timer_factory.is_none() {
                    // drop the message
                    return Ok(());
                }
                self.on_ack_message(&message);
            }
            slim_datapath::api::ProtoSessionMessageType::RtxRequest => {
                debug!("received rtx message");
                if self.timer_factory.is_none() {
                    // drop the message
                    return Ok(());
                }
                // receiving an rtx request for a message we stop the
                // corresponding ack timer. For this point on if the
                // message is not delivered the receiver side will keep
                // asking for it
                self.on_ack_message(&message);
                // after the ack removal, process the rtx request
                self.on_rtx_message(message).await?;
            }
            _ => {
                // TODO: Add missing message types (e.g. Channel messages)
                debug!("unexpected message type");
            }
        }

        // return the right state
        Ok(())
    }

    async fn on_publish_message(&mut self, mut message: Message) -> Result<(), SessionError> {
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

        // Set the session id, message id and session type
        header.set_message_id(message_id);
        header.set_session_id(self.session_id);
        header.set_session_type(self.session_type);

        if self.timer_factory.is_some() {
            debug!("reliable sender, set all timers");
            // add the message to the producer buffer. this is used to re-send
            // messages when acks are missing and to handle retrasmissions
            self.buffer.push(message.clone());

            // create a timer for the new packet and update the state
            let gt = GroupTimer {
                missing_timers: self.endpoints_list.clone(),
                timer: self.timer_factory.as_ref().unwrap().create_and_start_timer(
                    message_id,
                    message.get_session_message_type(),
                    None,
                ),
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

            // send the message
            self.tx
                .send_to_slim(Ok(msg))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()))
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
            );

            // send the message
            self.tx
                .send_to_slim(Ok(msg))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()))
        }
    }

    pub async fn on_timer_timeout(&mut self, id: u32) -> Result<(), SessionError> {
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

    pub async fn on_timer_failure(&mut self, id: u32) -> Result<(), SessionError> {
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

    pub fn add_endpoint(&mut self, endpoint: Name) {
        debug!("add endpoint {}", endpoint);
        // add endpoint to the list
        self.endpoints_list.insert(endpoint);
    }

    pub fn remove_endpoint(&mut self, endpoint: &Name) {
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

    pub fn check_drain_completion(&self) -> bool {
        // Drain is complete if we're draining and no pending acks remain
        if self.draining_state == SenderDrainStatus::Completed
            || self.draining_state == SenderDrainStatus::Initiated && self.pending_acks.is_empty()
        {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_datapath::api::ApplicationPayload;
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
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender.add_endpoint(remote.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &remote,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
            .on_message(message.clone())
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
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(10);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender.add_endpoint(remote.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &remote,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
                    .await
                    .expect("error handling timer failure");
            }
            _ => panic!("Expected TimerFailure signal, got: {:?}", signal),
        }

        // wait for timeout - should timeout since no more messages should be sent
        let res = timeout(Duration::from_millis(100), rx_slim.recv()).await;
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
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender.add_endpoint(remote.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &remote,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut ack = Message::new_publish(
            &remote,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );

        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender.on_message(ack).await.expect("error sending message");

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
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender.add_endpoint(remote1.clone());
        sender.add_endpoint(remote2.clone());
        sender.add_endpoint(remote3.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &group,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut ack1 = Message::new_publish(
            &remote1,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack2 = Message::new_publish(
            &remote2,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack2.get_session_header_mut().set_message_id(1);
        ack2.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::new_publish(
            &remote3,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send all 3 acks
        sender.on_message(ack1).await.expect("error sending ack1");
        sender.on_message(ack2).await.expect("error sending ack2");
        sender.on_message(ack3).await.expect("error sending ack3");

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
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender.add_endpoint(remote1.clone());
        sender.add_endpoint(remote2.clone());
        sender.add_endpoint(remote3.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &group,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut ack1 = Message::new_publish(
            &remote1,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::new_publish(
            &remote3,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender.on_message(ack1).await.expect("error sending ack1");
        sender.on_message(ack3).await.expect("error sending ack3");

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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender.add_endpoint(remote1.clone());
        sender.add_endpoint(remote2.clone());
        sender.add_endpoint(remote3.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &group,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut ack1 = Message::new_publish(
            &remote1,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::new_publish(
            &remote3,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack2)
        sender.on_message(ack1).await.expect("error sending ack1");
        sender.on_message(ack3).await.expect("error sending ack3");

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
        let (tx_app, _) = tokio::sync::mpsc::channel(10);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            tx,
            Some(tx_signal),
        );
        let group = Name::from_strings(["org", "ns", "group"]);
        let remote1 = Name::from_strings(["org", "ns", "remote1"]);
        let remote2 = Name::from_strings(["org", "ns", "remote2"]);
        let remote3 = Name::from_strings(["org", "ns", "remote3"]);

        sender.add_endpoint(remote1.clone());
        sender.add_endpoint(remote2.clone());
        sender.add_endpoint(remote3.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &group,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut ack1 = Message::new_publish(
            &remote1,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack1.get_session_header_mut().set_message_id(1);
        ack1.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        let mut ack3 = Message::new_publish(
            &remote3,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack3.get_session_header_mut().set_message_id(1);
        ack3.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        // Send only 2 out of 3 acks (missing ack from remote2)
        sender.on_message(ack1).await.expect("error sending ack1");
        sender.on_message(ack3).await.expect("error sending ack3");

        // Send RTX request from endpoint 2 instead of removing it
        let mut rtx_request = Message::new_publish(
            &remote2,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request.get_slim_header_mut().set_incoming_conn(Some(1));

        sender
            .on_message(rtx_request)
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
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        sender.add_endpoint(remote.clone());

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &remote,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        sender
            .on_message(message.clone())
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
        let mut rtx_request = Message::new_publish(
            &remote,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request
            .get_slim_header_mut()
            .set_incoming_conn(Some(123)); // simulate an incoming connection

        // Send the RTX request
        sender
            .on_message(rtx_request)
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
        let mut ack = Message::new_publish(
            &remote,
            &source,
            None,
            None,
            Some(ApplicationPayload::new("", vec![]).as_content()),
        );
        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender.on_message(ack).await.expect("error sending ack");

        // wait for timeout - should timeout since timer should be stopped after ack
        let res = timeout(Duration::from_millis(800), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_send_message_with_no_endpoints() {
        // send message without adding any endpoints, this should fail
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            tx,
            Some(tx_signal),
        );
        let remote = Name::from_strings(["org", "ns", "remote"]);

        // DO NOT add any endpoints - this is the key part of the test

        // Create a test message
        let source = Name::from_strings(["org", "ns", "source"]);
        let mut message = Message::new_publish(
            &source,
            &remote,
            None,
            None,
            Some(ApplicationPayload::new("test_payload", vec![1, 2, 3, 4]).as_content()),
        );

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function - this should fail
        let result = sender.on_message(message.clone()).await;

        // The on_message call should succeed (it just queues the message), but the internal processing should fail
        // We need to check that no message was actually sent to rx_slim
        assert!(result.is_err(), "the message send should fail");
    }
}
