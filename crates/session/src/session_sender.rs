// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use rand::Rng;
use slim_datapath::api::{Participant, ProtoSessionType};
use slim_datapath::messages::utils::{MAX_PUBLISH_ID, PUBLISH_TO};
use slim_datapath::{api::ProtoMessage as Message, api::ProtoName};
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tracing::debug;

use crate::{
    SessionError,
    common::{SessionMessage, SessionOutput},
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

struct GroupTimer {
    /// list of encoded names for which we did not get an ack yet
    missing_timers: HashSet<[u64; 3]>,

    /// the timer
    timer: Timer,
}

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
    pending_acks_per_endpoint: HashMap<[u64; 3], HashSet<u32>>,

    /// list of the endpoints in the conversation, keyed by [u64; 3] components for
    /// zero-alloc hot-path lookups; ProtoName value retained for retransmit
    /// set_destination calls (group retransmit must rewrite the destination).
    endpoints_list: HashMap<[u64; 3], ProtoName>,

    /// message id, used to send sequential messages
    /// it goes from 0 to 2^16. higher ids are used for messages
    /// sent using the publish_to function.
    next_id: u32,

    /// session id to had in the message header
    session_id: u32,

    /// session type to set in the message header
    session_type: ProtoSessionType,

    /// set to true if the sender is receiving packets
    /// to send but no one is connected to the session yet
    to_flush: bool,

    /// drain state - when true, no new messages from app are accepted
    draining_state: SenderDrainStatus,

    /// oneshot senders to signal when network acks are received for each message
    ack_notifiers: HashMap<u32, oneshot::Sender<Result<(), SessionError>>>,
}

impl SessionSender {
    const MAX_FANOUT: u32 = 256;

    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        session_type: ProtoSessionType,
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
            endpoints_list: HashMap::new(),
            next_id: 0,
            session_id,
            session_type,
            to_flush: false,
            draining_state: SenderDrainStatus::NotDraining,
            ack_notifiers: HashMap::new(),
        }
    }

    /// Send a message with optional acknowledgment notification
    pub fn on_message(
        &mut self,
        mut message: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
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
                    message.remove_metadata(PUBLISH_TO);
                }
                self.on_publish_message(message, ack_tx)
            }
            slim_datapath::api::ProtoSessionMessageType::MsgAck => {
                debug!("received ack message");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                if self.timer_factory.is_none() {
                    return Ok(SessionOutput::new());
                }
                self.on_ack_message(&message);
                Ok(SessionOutput::new())
            }
            slim_datapath::api::ProtoSessionMessageType::RtxRequest => {
                debug!("received rtx message");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                if self.timer_factory.is_none() {
                    return Ok(SessionOutput::new());
                }
                self.on_ack_message(&message);
                self.on_rtx_message(message)
            }
            _ => {
                debug!("unexpected message type");
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                Ok(SessionOutput::new())
            }
        }
    }

    fn on_publish_message(
        &mut self,
        mut message: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        let is_publish_to = message.metadata.contains_key(PUBLISH_TO);

        if is_publish_to && !self.endpoints_list.contains_key(&message.get_encoded_dst().0) {
            let dst = message.get_dst();
            debug!(
                %dst,
                "cannot forward the message to the select destination",
            );
            return Err(SessionError::UnknownDestination(dst));
        }

        let (message_id, fanout) = self.id_and_fanout(is_publish_to);

        let session_header = message.get_session_header_mut();
        session_header.set_message_id(message_id);
        session_header.set_session_id(self.session_id);
        session_header.set_session_type(self.session_type);
        message.get_slim_header_mut().set_fanout(fanout);

        if let Some(tx) = ack_tx {
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
            if !is_publish_to {
                self.buffer.push(message);
            }
            self.to_flush = true;
            return Ok(SessionOutput::new());
        }

        if self.timer_factory.is_some() && !is_publish_to {
            self.buffer.push(message.clone());
        }

        self.set_timer_and_send(message)
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

    fn set_timer_and_send(&mut self, message: Message) -> Result<SessionOutput, SessionError> {
        let message_id = message.get_id();
        let is_publish_to = message.metadata.contains_key(PUBLISH_TO);
        debug!(%message_id, "send new message");

        if let Some(timer_factory) = &self.timer_factory {
            debug!("reliable sender, set all timers");
            let missing_timers: HashSet<[u64; 3]> = if is_publish_to {
                HashSet::from([message.get_encoded_dst().0])
            } else {
                self.endpoints_list.keys().copied().collect()
            };

            let gt = GroupTimer {
                missing_timers,
                timer: timer_factory.create_and_start_timer(
                    message_id,
                    message.get_session_message_type(),
                    None,
                ),
            };

            let endpoints_to_track: Vec<[u64; 3]> = if is_publish_to {
                self.pending_acks
                    .insert(message_id, (gt, Some(message.clone())));
                vec![message.get_encoded_dst().0]
            } else {
                self.pending_acks.insert(message_id, (gt, None));
                self.endpoints_list.keys().copied().collect()
            };

            for enc in &endpoints_to_track {
                debug!(%message_id, "add timer for message");
                if let Some(acks) = self.pending_acks_per_endpoint.get_mut(enc) {
                    acks.insert(message_id);
                } else {
                    let mut set = HashSet::new();
                    set.insert(message_id);
                    self.pending_acks_per_endpoint.insert(*enc, set);
                }
            }
        }

        debug!("send message");
        let mut output = SessionOutput::new();
        output.push_slim(message);
        Ok(output)
    }

    fn on_ack_message(&mut self, message: &Message) {
        let encoded_source = message.get_encoded_source().0;
        let message_id = message.get_id();
        debug!(%message_id, source = %message.get_source(), "received ack message");

        let mut delete = false;
        // remove the source from the pending acks
        // notice that the state for this ack id may not exist if all the endpoints
        // associated to it where removed before getting the acks
        if let Some((gt, _m)) = self.pending_acks.get_mut(&message_id) {
            debug!("try to remove from pending acks");
            gt.missing_timers.remove(&encoded_source);
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
        if let Some(set) = self.pending_acks_per_endpoint.get_mut(&encoded_source) {
            debug!(
                %message_id,
                "remove message from pending acks for endpoint"
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

    fn on_rtx_message(&mut self, message: Message) -> Result<SessionOutput, SessionError> {
        let source_proto = message.get_slim_header().get_source();
        let message_id = message.get_id();
        let incoming_conn = message.get_incoming_conn();

        debug!(
            id = %message_id,
            source = %message.get_source(),
            "received rtx request",
        );

        let mut output = SessionOutput::new();
        if let Some(mut msg) = self.buffer.get(message_id as usize) {
            debug!("the message is still exists, send it as rtx reply");
            msg.get_slim_header_mut().set_destination(source_proto.clone());
            msg.get_slim_header_mut()
                .set_forward_to(Some(incoming_conn));
            msg.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply);

            output.push_slim(msg);
        } else {
            debug!("the message does not exists anymore, send and error");
            let dest_proto = message.get_slim_header().get_dst();
            let msg = Message::builder()
                .source(dest_proto)
                .destination(source_proto)
                .identity("")
                .forward_to(incoming_conn)
                .session_type(message.get_session_type())
                .session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxReply)
                .session_id(self.session_id)
                .message_id(message_id)
                .application_payload("", vec![])
                .error(true)
                .build_publish()?;

            output.push_slim(msg);
        }
        Ok(output)
    }

    pub fn on_timer_timeout(&mut self, id: u32) -> Result<SessionOutput, SessionError> {
        debug!(%id, "message timeout");

        if id > MAX_PUBLISH_ID {
            if let Some((_gt, Some(msg))) = self.pending_acks.get_mut(&id) {
                let mut output = SessionOutput::new();
                output.push_slim(msg.clone());
                return Ok(output);
            } else {
                return self.on_timer_failure(id);
            }
        }

        if let Some(message) = self.buffer.get(id as usize) {
            debug!("the message is still in the buffer, try to send it again to all the remotes");
            let mut output = SessionOutput::new();
            if let Some((gt, _)) = self.pending_acks.get(&id) {
                let missing: Vec<[u64; 3]> = gt.missing_timers.iter().copied().collect();
                for enc in &missing {
                    if let Some(name) = self.endpoints_list.get(enc) {
                        debug!(%id, dst = %name, "resend message");
                        let mut m = message.clone();
                        m.get_slim_header_mut().set_destination(name.clone());
                        output.push_slim(m);
                    }
                }
            }
            Ok(output)
        } else {
            debug!(
                %id,
                "message not in the buffer anymore, delete the associated timer",
            );
            self.on_timer_failure(id)
        }
    }

    pub fn on_failure(
        &mut self,
        id: u32,
        error: SessionError,
    ) -> Result<SessionOutput, SessionError> {
        // remove all the state related to this timer
        if let Some((gt, _)) = self.pending_acks.get_mut(&id) {
            for enc in &gt.missing_timers {
                if let Some(set) = self.pending_acks_per_endpoint.get_mut(enc) {
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

        Ok(SessionOutput::new())
    }

    pub fn on_timer_failure(&mut self, id: u32) -> Result<SessionOutput, SessionError> {
        debug!(%id, "timer failure, clear state");
        self.on_failure(id, SessionError::MessageSendRetryFailed { id })
    }

    pub fn on_slim_failure(&mut self, error: SessionError) -> Result<SessionOutput, SessionError> {
        let Some(session_ctx) = error.session_context() else {
            return Err(SessionError::UnexpectedError {
                source: Box::new(error),
            });
        };
        let message_id = session_ctx.message_id;
        debug!(%message_id, "slim reported failure, clear state");
        self.on_failure(message_id, error)
    }

    pub fn add_endpoint(&mut self, endpoint: &Participant) -> Result<SessionOutput, SessionError> {
        let settings = endpoint
            .settings
            .as_ref()
            .ok_or(SessionError::MalformedParticipant)?;
        let name = endpoint
            .name
            .as_ref()
            .ok_or(SessionError::MalformedParticipant)?
            .clone();

        if !settings.is_receiver() {
            debug!(%name, "new participant will not receive data messages, do not add it to the endpoint list");
            return Ok(SessionOutput::new());
        }

        self.endpoints_list.insert(name.components(), name.clone());

        debug!(
            %name,
            list_len = %self.endpoints_list.len(),
            "add endpoint",
        );

        if self.to_flush && self.endpoints_list.len() == 1 {
            self.to_flush = false;
            let mut output = SessionOutput::new();
            let messages: Vec<_> = self.buffer.iter().cloned().collect();
            for p in messages {
                let flush_output = self.set_timer_and_send(p)?;
                output.extend(flush_output);
            }
            return Ok(output);
        }

        Ok(SessionOutput::new())
    }

    pub fn remove_endpoint(&mut self, endpoint: &ProtoName) {
        debug!(
            %endpoint,
            list_len = %self.endpoints_list.len(),
            "remove endpoint",
        );
        let key = endpoint.components();
        // remove endpoint from the list and remove all the ack state
        // notice that no ack state may be associated to the endpoint
        // (e.g. endpoint added but no message sent)
        if let Some(set) = self.pending_acks_per_endpoint.get(&key) {
            for id in set {
                let mut delete = false;
                if let Some((gt, _)) = self.pending_acks.get_mut(id) {
                    debug!(
                        %id, %endpoint,
                        "try to remove timer",
                    );
                    gt.missing_timers.remove(&key);
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
        self.pending_acks_per_endpoint.remove(&key);
        self.endpoints_list.remove(&key);
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

    pub fn participants_list(&self) -> Vec<ProtoName> {
        self.endpoints_list.values().cloned().collect()
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
    use crate::common::OutboundMessage;

    use super::*;
    use slim_datapath::api::ParticipantSettings;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    /// Helper to extract ToSlim messages from SessionOutput
    fn extract_slim_messages(output: SessionOutput) -> Vec<Message> {
        output
            .messages
            .into_iter()
            .filter_map(|m| match m {
                OutboundMessage::ToSlim(msg) => Some(msg),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app() {
        // send messages from the app and verify that they arrive correctly formatted
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        // Create a test message
        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        // Set session message type to Msg for reliable sender
        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message using on_message function
        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(msgs[0].get_id(), 1);

        // Send another message
        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::Msg
        );
        assert_eq!(msgs[0].get_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_timeout() {
        // send message from the app and check for timeouts
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        // Create a test message
        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message
        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);

        // Wait for first timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                let output = sender
                    .on_timer_timeout(message_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), 1);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for second timeout signal and handle it
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                let output = sender
                    .on_timer_timeout(message_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), 1);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

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
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_ack() {
        // send message from the app and get an ack so no timer should be triggered
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        // Create a test message
        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send the message
        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);

        // receive an ack from the remote
        let mut ack = Message::builder()
            .source(remote_name.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender.on_message(ack, None).expect("error sending ack");

        // wait for timeout - should timeout since no timer should trigger after the ack
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_ack_3_endpoints() {
        // send message to 3 endpoints and get acks from all 3
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote3_name = ProtoName::from_strings(["org", "ns", "remote3"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());
        let remote3 = Participant::new(remote3_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(ProtoName::from_strings(["org", "ns", "group"]))
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);

        // receive acks from all 3 remotes
        for remote_name in [&remote1_name, &remote2_name, &remote3_name] {
            let mut ack = Message::builder()
                .source(remote_name.clone())
                .destination(source.clone())
                .application_payload("", vec![])
                .build_publish()
                .unwrap();
            ack.get_session_header_mut().set_message_id(1);
            ack.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
            sender.on_message(ack, None).expect("error sending ack");
        }

        // wait for timeout - should timeout since no timer should trigger after all acks
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_3_endpoints() {
        // send message to 3 endpoints but only get acks from 2
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote3_name = ProtoName::from_strings(["org", "ns", "remote3"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());
        let remote3 = Participant::new(remote3_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(ProtoName::from_strings(["org", "ns", "group"]))
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);

        // receive acks from only remote1 and remote3
        for remote_name in [&remote1_name, &remote3_name] {
            let mut ack = Message::builder()
                .source(remote_name.clone())
                .destination(source.clone())
                .application_payload("", vec![])
                .build_publish()
                .unwrap();
            ack.get_session_header_mut().set_message_id(1);
            ack.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
            sender.on_message(ack, None).expect("error sending ack");
        }

        // Wait for first timeout signal
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                let output = sender
                    .on_timer_timeout(message_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), 1);
                assert_eq!(retransmitted[0].get_dst(), remote2_name);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for second timeout signal
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                let output = sender
                    .on_timer_timeout(message_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), 1);
                assert_eq!(retransmitted[0].get_dst(), remote2_name);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // Wait for timer failure signal
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer failure signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerFailure { .. } => {}
            _ => panic!("Expected TimerFailure signal, got: {:?}", signal),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_removing_endpoint() {
        // send message to 3 endpoints, get acks from 2, remove the missing one
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote3_name = ProtoName::from_strings(["org", "ns", "remote3"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());
        let remote3 = Participant::new(remote3_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(ProtoName::from_strings(["org", "ns", "group"]))
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        // receive acks from only remote1 and remote3
        for remote_name in [&remote1_name, &remote3_name] {
            let mut ack = Message::builder()
                .source(remote_name.clone())
                .destination(source.clone())
                .application_payload("", vec![])
                .build_publish()
                .unwrap();
            ack.get_session_header_mut().set_message_id(1);
            ack.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
            sender.on_message(ack, None).expect("error sending ack");
        }

        // remove endpoint 2
        sender.remove_endpoint(&remote2_name);

        // wait for timeout - should expire since timer was cleaned up
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_message_from_app_with_partial_acks_rtx_request() {
        // send message to 3 endpoints, get acks from 2, then RTX request from endpoint 2
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote3_name = ProtoName::from_strings(["org", "ns", "remote3"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());
        let remote3 = Participant::new(remote3_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(ProtoName::from_strings(["org", "ns", "group"]))
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        // receive acks from only remote1 and remote3
        for remote_name in [&remote1_name, &remote3_name] {
            let mut ack = Message::builder()
                .source(remote_name.clone())
                .destination(source.clone())
                .application_payload("", vec![])
                .build_publish()
                .unwrap();
            ack.get_session_header_mut().set_message_id(1);
            ack.get_session_header_mut()
                .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
            sender.on_message(ack, None).expect("error sending ack");
        }

        // Send RTX request from endpoint 2
        let mut rtx_request = Message::builder()
            .source(remote2_name.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        rtx_request.get_session_header_mut().set_message_id(1);
        rtx_request
            .get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest);
        rtx_request.get_slim_header_mut().set_incoming_conn(Some(1));

        let output = sender
            .on_message(rtx_request, None)
            .expect("error sending rtx_request");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxReply
        );
        assert_eq!(msgs[0].get_id(), 1);
        assert_eq!(msgs[0].get_dst(), remote2_name);

        // wait for timeout - timers should be stopped after RTX ack
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_request_and_reply_with_ack() {
        // send message, no ack, timer triggers, RTX request, reply, then ack
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);

        // Wait for timeout signal
        let signal = timeout(Duration::from_millis(800), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match signal {
            crate::common::SessionMessage::TimerTimeout { message_id, .. } => {
                let output = sender
                    .on_timer_timeout(message_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), 1);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }

        // RTX request from the remote
        let mut rtx_request = Message::builder()
            .source(remote_name.clone())
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

        let output = sender
            .on_message(rtx_request, None)
            .expect("error sending rtx_request");
        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::RtxReply
        );
        assert_eq!(msgs[0].get_id(), 1);
        assert_eq!(msgs[0].get_dst(), remote_name);
        assert_eq!(msgs[0].get_slim_header().forward_to(), 123);

        // ack from the remote
        let mut ack = Message::builder()
            .source(remote_name.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(1);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);

        sender.on_message(ack, None).expect("error sending ack");

        // wait for timeout - should timeout since timer stopped after ack
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_send_message_with_no_endpoints() {
        // send message without endpoints, then add one — buffered message should flush
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        // Send — should buffer (no output)
        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");
        assert!(output.is_empty());

        // add endpoint — should flush
        let output = sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_multicast_session() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote3_name = ProtoName::from_strings(["org", "ns", "remote3"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());
        let remote3 = Participant::new(remote3_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote3)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let output = sender
            .on_message(message.clone(), None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_slim_header().get_fanout(), 1);
        assert_eq!(msgs[0].get_dst(), remote2_name);
        assert!(sender.buffer.get(msgs[0].get_id() as usize).is_none());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_with_unknown_destination() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let unknown_remote_name = ProtoName::from_strings(["org", "ns", "unknown"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(unknown_remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let result = sender.on_message(message, None);

        assert!(
            result.is_err_and(|e| { matches!(e, SessionError::UnknownDestination(_)) }),
            "Expected UnknownDestination error",
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_p2p_session_fallback() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let output = sender
            .on_message(message, None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);
        assert!(sender.buffer.get(1).is_some());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_ack_handling() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let output = sender
            .on_message(message, None)
            .expect("error sending message");

        let msgs = extract_slim_messages(output);
        let message_id = msgs[0].get_id();

        // Send an ack
        let mut ack = Message::builder()
            .source(remote2_name.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(message_id);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
        ack.metadata.insert(PUBLISH_TO.to_string(), String::new());

        sender.on_message(ack, None).expect("error sending ack");

        assert!(!sender.pending_acks.contains_key(&message_id));

        // No timers should trigger
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_timeout_retransmission() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let output = sender
            .on_message(message, None)
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        let message_id = msgs[0].get_id();

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
                let output = sender
                    .on_timer_timeout(timer_id)
                    .expect("error handling timeout");
                let retransmitted = extract_slim_messages(output);
                assert_eq!(retransmitted.len(), 1);
                assert_eq!(retransmitted[0].get_id(), message_id);
                assert_eq!(retransmitted[0].get_dst(), remote2_name);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", signal),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_endpoint_removed_before_timeout() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote2_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        sender
            .on_message(message, None)
            .expect("error sending message");

        // Remove the endpoint before timeout
        sender.remove_endpoint(&remote2_name);

        // No timer signal should arrive
        let res = timeout(Duration::from_millis(800), rx_signal.recv()).await;
        assert!(res.is_err(), "Expected no timer signal but got: {:?}", res);

        assert!(sender.pending_acks.is_empty());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ack_notifiers_with_publish_to() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        message
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let (ack_tx, mut ack_rx) = oneshot::channel();

        let output = sender
            .on_message(message, Some(ack_tx))
            .expect("error sending message");
        let msgs = extract_slim_messages(output);
        let message_id = msgs[0].get_id();

        // Send an ack
        let mut ack = Message::builder()
            .source(remote_name.clone())
            .destination(source.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack.get_session_header_mut().set_message_id(message_id);
        ack.get_session_header_mut()
            .set_session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck);
        ack.metadata.insert(PUBLISH_TO.to_string(), String::new());

        sender.on_message(ack, None).expect("error sending ack");

        // Verify ack notifier receives success
        let ack_result = ack_rx.try_recv().expect("ack notification not received");
        assert!(ack_result.is_ok());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_mixed_normal_and_publish_to_messages() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(1);

        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut sender = SessionSender::new(
            Some(settings),
            10,
            ProtoSessionType::Multicast,
            Some(tx_signal),
        );
        let group = ProtoName::from_strings(["org", "ns", "group"]);
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);
        let remote1 = Participant::new(remote1_name.clone(), ParticipantSettings::bidirectional());
        let remote2 = Participant::new(remote2_name.clone(), ParticipantSettings::bidirectional());

        sender
            .add_endpoint(&remote1)
            .expect("error adding participant");
        sender
            .add_endpoint(&remote2)
            .expect("error adding participant");

        let source = ProtoName::from_strings(["org", "ns", "source"]);

        // Send a normal multicast message
        let mut normal_msg = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .application_payload("normal", vec![1])
            .build_publish()
            .unwrap();
        normal_msg.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);

        let output = sender
            .on_message(normal_msg, None)
            .expect("error sending normal message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get_id(), 1);
        assert_eq!(
            msgs[0].get_slim_header().get_fanout(),
            SessionSender::MAX_FANOUT
        );

        // Send a PUBLISH_TO message
        let mut publish_to_msg = Message::builder()
            .source(source.clone())
            .destination(remote1_name.clone())
            .application_payload("publish_to", vec![2])
            .build_publish()
            .unwrap();
        publish_to_msg.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
        publish_to_msg
            .metadata
            .insert(PUBLISH_TO.to_string(), String::new());

        let output = sender
            .on_message(publish_to_msg, None)
            .expect("error sending publish_to message");

        let msgs = extract_slim_messages(output);
        assert_eq!(msgs.len(), 1);
        assert_ne!(msgs[0].get_id(), 2); // Random ID
        assert_eq!(msgs[0].get_slim_header().get_fanout(), 1);
        assert_eq!(msgs[0].get_dst(), remote1_name);

        assert!(sender.buffer.get(1).is_some());
        assert!(sender.buffer.get(msgs[0].get_id() as usize).is_none());
    }
}
