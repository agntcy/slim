// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use slim_datapath::{api::ProtoMessage as Message, messages::Name};
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::{
    SessionError, Transmitter,
    common::SessionMessage,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
    transmitter::SessionTransmitter,
};

/// used a result in OnMessage function
#[derive(PartialEq, Clone, Debug)]
enum ControllerSenderDrainStatus {
    NotDraining,
    Initiated,
    Completed,
}

struct PendingReply {
    /// Missing replies
    /// Keep track of the names so that if we get  multiple acks from
    /// the same endpoint we don't count it twice
    missing_replies: HashSet<Name>,

    /// Message to resend in case of timeout
    message: Message,

    /// the timer
    timer: Timer,
}

pub struct ControllerSender {
    /// timer factory to crate timers for acks
    timer_factory: TimerFactory,

    /// local name to be removed in the missing replies set
    local_name: Name,

    /// list of pending replies for each control message
    pending_replies: HashMap<u32, PendingReply>,

    /// send packets to slim or the app
    tx: SessionTransmitter,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ControllerSenderDrainStatus,
}

impl ControllerSender {
    pub fn new(
        timer_settings: TimerSettings,
        local_name: Name,
        tx: SessionTransmitter,
        tx_signals: Sender<SessionMessage>,
    ) -> Self {
        ControllerSender {
            timer_factory: TimerFactory::new(timer_settings, tx_signals),
            local_name: {
                // reset the id to avoid match problem
                let mut name = local_name;
                name.reset_id();
                name
            },
            pending_replies: HashMap::new(),
            tx,
            draining_state: ControllerSenderDrainStatus::NotDraining,
        }
    }

    pub async fn on_message(&mut self, message: &Message) -> Result<(), SessionError> {
        if self.draining_state == ControllerSenderDrainStatus::Completed {
            return Err(SessionError::Processing(
                "sender closed, drop message".to_string(),
            ));
        }

        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest
            | slim_datapath::api::ProtoSessionMessageType::JoinRequest
            | slim_datapath::api::ProtoSessionMessageType::LeaveRequest
            | slim_datapath::api::ProtoSessionMessageType::GroupWelcome => {
                if self.draining_state == ControllerSenderDrainStatus::Initiated {
                    // draining period is started, do no accept any new message
                    return Err(SessionError::Processing(
                        "draining period started, do not accept new messages".to_string(),
                    ));
                }
                let mut missing_replies = HashSet::new();
                let mut name = message.get_dst();
                name.reset_id();
                missing_replies.insert(name);
                self.on_send_message(message, missing_replies).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::GroupClose => {
                let payload = message.extract_group_close().map_err(|e| {
                    SessionError::Processing(format!(
                        "failed to extract group close payload: {}",
                        e
                    ))
                })?;
                let mut missing_replies = HashSet::new();
                for n in &payload.participants {
                    let name = Name::from(n);
                    missing_replies.insert(name);
                }
                self.on_send_message(message, missing_replies).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::DiscoveryReply
            | slim_datapath::api::ProtoSessionMessageType::JoinReply
            | slim_datapath::api::ProtoSessionMessageType::LeaveReply
            | slim_datapath::api::ProtoSessionMessageType::GroupAck => {
                self.on_reply_message(message);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupNack => {
                // in case on Nack we stop the timer as for the Acks
                // and we leave the application/controller decide what
                // to do to handle it
                self.on_reply_message(message);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupAdd => {
                // compute the list of participants that needs to send an ack
                let payload = message.extract_group_add().map_err(|e| {
                    SessionError::Processing(format!("failed to extract group add payload: {}", e))
                })?;
                let mut missing_replies = HashSet::new();
                let mut new_participant = Name::from(payload.new_participant.as_ref().ok_or(
                    SessionError::Processing(
                        "missing new participant in GroupAdd message".to_string(),
                    ),
                )?);
                new_participant.reset_id();
                for p in &payload.participants {
                    // exclude the local name and the new participant
                    let mut name = Name::from(p);
                    name.reset_id();
                    if name != self.local_name && name != new_participant {
                        missing_replies.insert(name);
                    }
                }
                self.on_send_message(message, missing_replies).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::GroupRemove => {
                // compute the list of participants that needs to send an ack
                let payload = message.extract_group_remove().map_err(|e| {
                    SessionError::Processing(format!(
                        "failed to extract group remove payload: {}",
                        e
                    ))
                })?;

                let mut missing_replies = HashSet::new();
                for p in &payload.participants {
                    // exclude only the local name
                    let mut name = Name::from(p);
                    name.reset_id();
                    if name != self.local_name {
                        missing_replies.insert(name);
                    }
                }

                // also the message that we are removing will get the update
                // so we need to add it in the list of endpoint from where
                // we expected to receive an ack
                let to_remove = Name::from(payload.removed_participant.as_ref().ok_or(
                    SessionError::Processing(
                        "missing removed participant in GroupRemove message".to_string(),
                    ),
                )?);
                if to_remove != self.local_name {
                    missing_replies.insert(to_remove);
                }

                self.on_send_message(message, missing_replies).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::GroupProposal => todo!(),
            _ => {
                debug!("unexpected message type");
            }
        }

        Ok(())
    }

    async fn on_send_message(
        &mut self,
        message: &Message,
        missing_replies: HashSet<Name>,
    ) -> Result<(), SessionError> {
        let id = message.get_id();

        debug!(
            "create a new timer for message {}, waiting response from {:?}",
            id, missing_replies
        );
        let pending = PendingReply {
            missing_replies,
            message: message.clone(),
            timer: self.timer_factory.create_and_start_timer(
                id,
                message.get_session_message_type(),
                None,
            ),
        };

        self.pending_replies.insert(id, pending);
        self.tx
            .send_to_slim(Ok(message.clone()))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    fn on_reply_message(&mut self, message: &Message) {
        let id = message.get_id();
        debug!(
            "receive reply for message {} from {}",
            id,
            message.get_source()
        );

        let mut delete = false;
        if let Some(pending) = self.pending_replies.get_mut(&id) {
            debug!("try to remove {} from pending acks", id);
            let mut name = message.get_source();
            name.reset_id();
            pending.missing_replies.remove(&name);
            if pending.missing_replies.is_empty() {
                debug!("all replies received, remove timer");
                pending.timer.stop();
                delete = true;
            }
        }

        if delete {
            self.pending_replies.remove(&id);
        }
    }

    pub fn is_still_pending(&self, message_id: u32) -> bool {
        self.pending_replies.contains_key(&message_id)
    }

    pub async fn on_timer_timeout(&mut self, id: u32) -> Result<(), SessionError> {
        debug!("timeout for message {}", id);

        if let Some(pending) = self.pending_replies.get(&id) {
            return self
                .tx
                .send_to_slim(Ok(pending.message.clone()))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()));
        };

        Err(SessionError::SlimTransmission(format!(
            "timer {} does not exists",
            id
        )))
    }

    pub async fn on_timer_failure(&mut self, id: u32) {
        debug!("Timer failure for message {}", id);

        if let Some(gt) = self.pending_replies.get_mut(&id) {
            gt.timer.stop();
        }
        self.pending_replies.remove(&id);
    }

    pub fn clear_timers(&mut self) {
        for (_, mut p) in self.pending_replies.drain() {
            p.timer.stop();
        }
        self.pending_replies.clear();
    }

    pub fn start_drain(&mut self) {
        // set only initiated to true because we may need send request leave
        debug!("controller sender drain initiated");
        self.draining_state = ControllerSenderDrainStatus::Initiated;
    }

    pub fn drain_completed(&self) -> bool {
        // Drain is complete if we're draining and no pending acks remain
        if self.draining_state == ControllerSenderDrainStatus::Completed
            || self.draining_state == ControllerSenderDrainStatus::Initiated
                && self.pending_replies.is_empty()
        {
            return true;
        }
        false
    }

    pub fn close(&mut self) {
        self.clear_timers();
        self.draining_state = ControllerSenderDrainStatus::Completed;
    }
}

#[cfg(test)]
mod tests {
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_datapath::{
        api::{CommandPayload, ProtoSessionMessageType, ProtoSessionType},
        messages::Name,
    };
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_on_discovery_request() {
        // send a discovery request, wait for a retransmission of the message and then get the discovery reply
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a discovery request message
        let payload = CommandPayload::builder().discovery_request(None);
        let session_id = 1;

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&request)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the request");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Create the discovery reply
        let payload = CommandPayload::builder().discovery_reply();

        let reply = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryReply)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&reply)
            .await
            .expect("error sending reply");

        // this should stop the timer so we should not get any other message in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_join_request() {
        // send a join request, wait for a retransmission of the message and then get the join reply
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a join request message
        let payload = CommandPayload::builder().join_request(
            false, // enable_mls
            None,  // max_retries
            None,  // timer_duration
            None,  // channel
        );

        let session_id = 1;

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&request)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::JoinRequest);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the request");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Create the join reply
        let payload = CommandPayload::builder().join_reply(None);

        let reply = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the reply using on_message function
        sender
            .on_message(&reply)
            .await
            .expect("error sending reply");

        // this should stop the timer so we should not get any other message in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_leave_request() {
        // send a leave request, wait for a retransmission of the message and then get the leave reply
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a leave request message
        let payload = CommandPayload::builder().leave_request(None);

        let session_id = 1;

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&request)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::LeaveRequest);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the request");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, request);

        // Create the leave reply
        let payload = CommandPayload::builder().leave_reply();

        let reply = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveReply)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the reply using on_message function
        sender
            .on_message(&reply)
            .await
            .expect("error sending reply");

        // this should stop the timer so we should not get any other message in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_welcome() {
        // send a group welcome, wait for a retransmission of the message and then get the group ack
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);
        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a group welcome message
        let participant = Name::from_strings(["org", "ns", "participant"]);
        let payload = CommandPayload::builder()
            .group_welcome(vec![participant.clone(), source.clone()], None);
        let session_id = 1;

        let welcome = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&welcome)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, welcome);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::GroupWelcome);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the welcome");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, welcome);

        // Create the group ack
        let payload = CommandPayload::builder().group_ack();

        let ack = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the ack using on_message function
        sender.on_message(&ack).await.expect("error sending ack");

        // this should stop the timer so we should not get any other message in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_add_message() {
        // send a group add with 2 participants, wait for retransmission, then send 2 group acks
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a group add message with 2 participants
        let participant1 = Name::from_strings(["org", "ns", "participant1"]);
        let participant2 = Name::from_strings(["org", "ns", "participant2"]);
        let payload = CommandPayload::builder().group_add(
            participant1.clone(),
            vec![participant1.clone(), participant2.clone(), source.clone()],
            None, // mls_commit
        );

        let session_id = 1;

        let update = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAdd)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&update)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, update);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the add");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, update);

        // Create the first group ack from participant1
        let payload = CommandPayload::builder().group_ack();

        let ack1 = Message::builder()
            .source(participant1.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the first ack using on_message function
        sender.on_message(&ack1).await.expect("error sending ack");

        // Verify the message is still pending (timer should NOT stop yet with only 1/2 acks)
        assert!(
            sender.is_still_pending(1),
            "Message should still be pending after first ack"
        );

        // Create the second group ack from participant2
        let payload = CommandPayload::builder().group_ack();

        let ack2 = Message::builder()
            .source(participant2.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the second ack using on_message function
        sender.on_message(&ack2).await.expect("error sending ack");

        // Now the timer should be stopped - verify no more messages in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_update_duplicate_acks() {
        // send a group add with 2 participants, receive duplicate acks from same participant
        // verify timer doesn't stop until we get acks from BOTH different participants
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);

        let mut sender = ControllerSender::new(settings, source.clone(), tx, tx_signal);

        // Create a group add message with 2 participants (plus source)
        let participant1 = Name::from_strings(["org", "ns", "participant1"]);
        let participant2 = Name::from_strings(["org", "ns", "participant2"]);
        let payload = CommandPayload::builder().group_add(
            participant1.clone(),
            vec![participant1.clone(), participant2.clone(), source.clone()],
            None, // mls
        );

        let session_id = 1;

        let update = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAdd)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the message using on_message function
        sender
            .on_message(&update)
            .await
            .expect("error sending message");

        // Wait for the message to arrive at rx_slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, update);

        // Wait longer than the timer duration to account for async task scheduling
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify we got the right timeout
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // notify the timeout to the sender
        sender
            .on_timer_timeout(1)
            .await
            .expect("error re-sending the add");

        // Wait for the message to be sent again to slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error message");

        // Verify the message received is the right one
        assert_eq!(received, update);

        // Create the first group ack from participant1
        let payload = CommandPayload::builder().group_ack();

        let ack1 = Message::builder()
            .source(participant1.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the first ack using on_message function
        sender.on_message(&ack1).await.expect("error sending ack");

        // Verify the message is still pending (timer should NOT stop yet with only 1/2 acks)
        assert!(
            sender.is_still_pending(1),
            "Message should still be pending after first ack"
        );

        // Send the SAME ack again from participant1 (duplicate)
        sender
            .on_message(&ack1)
            .await
            .expect("error sending duplicate ack");

        // Verify the message is STILL pending - duplicate ack should not count
        assert!(
            sender.is_still_pending(1),
            "Message should still be pending after duplicate ack from same participant"
        );

        // Wait for another timeout since timer should still be running
        let timeout_msg = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for second timer signal - duplicate ack should not have stopped timer")
            .expect("channel closed");

        // Verify we got another timeout (timer didn't stop)
        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, 1);
                assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);
            }
            _ => panic!("Expected TimerTimeout message"),
        }

        // Now send ack from participant2 (the second unique participant)
        let payload = CommandPayload::builder().group_ack();

        let ack2 = Message::builder()
            .source(participant2.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        // Send the second ack from participant2
        sender.on_message(&ack2).await.expect("error sending ack");

        // NOW the timer should be stopped - verify no more messages in slim
        let res = timeout(Duration::from_millis(400), rx_slim.recv()).await;
        assert!(res.is_err(), "Expected timeout but got: {:?}", res);

        // Verify no more timeout signals
        let res = timeout(Duration::from_millis(100), rx_signal.recv()).await;
        assert!(
            res.is_err(),
            "Expected no more timeout signals after all unique acks received"
        );
    }
}
