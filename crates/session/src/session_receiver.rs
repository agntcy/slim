// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use slim_datapath::api::{EncodedName, ProtoMessage as Message, ProtoName, ProtoSessionType};
use slim_datapath::messages::utils::{PUBLISH_TO, TRUE_VAL};
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::{
    SessionError,
    common::{SessionMessage, SessionOutput},
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
    name: EncodedName,
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

pub struct SessionReceiver {
    /// buffer with received packets one per endpoint, keyed by EncodedName
    /// for zero-alloc hot-path lookups
    buffer: HashMap<EncodedName, ReceiverBuffer>,

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
    local_name: ProtoName,

    /// session type
    session_type: ProtoSessionType,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ReceiverDrainStatus,
}

impl SessionReceiver {
    /// to create the timer factory and send rtx messages
    /// timer_settings and tx_timer must be not null
    pub fn new(
        timer_settings: Option<TimerSettings>,
        session_id: u32,
        local_name: ProtoName,
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

        SessionReceiver {
            buffer: HashMap::new(),
            pending_rtxs: HashMap::new(),
            timer_factory: factory,
            session_id,
            session_type,
            local_name,
            draining_state: ReceiverDrainStatus::NotDraining,
        }
    }

    pub fn on_message(&mut self, mut message: Message) -> Result<SessionOutput, SessionError> {
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

                let mut output = self.build_ack(&message)?;
                let publish_output = self.on_publish_message(message)?;
                output.extend(publish_output);
                Ok(output)
            }
            slim_datapath::api::ProtoSessionMessageType::RtxReply => {
                debug!("received rtx message");
                self.on_rtx_message(message)
            }
            _ => {
                // TODO: Add missing message types (e.g. Channel messages)
                debug!("unexpected message type");
                Ok(SessionOutput::new())
            }
        }
    }

    pub fn on_publish_message(&mut self, message: Message) -> Result<SessionOutput, SessionError> {
        if self.timer_factory.is_none() || message.contains_metadata(PUBLISH_TO) {
            debug!(
                id = %message.get_id(),
                source = %message.get_source(),
                "received message, send it to the app without reordering",
            );
            let mut output = SessionOutput::new();
            output.push_app(Ok(message));
            return Ok(output);
        }

        let source_proto = message.get_slim_header().source.clone().unwrap();
        let in_conn = message.get_incoming_conn();
        let buffer = self.buffer.entry(source_proto.name.unwrap()).or_default();

        let (recv_vec, rtx_vec) = buffer.on_received_message(message);
        self.handle_recv_and_rtx_vectors(&source_proto, in_conn, recv_vec, rtx_vec)
    }

    pub fn build_ack(&self, message: &Message) -> Result<SessionOutput, SessionError> {
        // we need to send an ack message only if the factory is not none
        // in this case the session is reliable and the producer is expecting acks
        if self.timer_factory.is_none() {
            return Ok(SessionOutput::new());
        }

        let publish_meta = message.contains_metadata(PUBLISH_TO).then(|| {
            std::iter::once((PUBLISH_TO.to_string(), TRUE_VAL.to_string()))
                .collect::<std::collections::HashMap<_, _>>()
        });

        // Pass the source ProtoName directly as ACK destination — avoids ProtoName → Name → ProtoName roundtrip.
        let source_proto = message.get_slim_header().source.clone().unwrap();
        let mut builder = Message::builder()
            .source(self.local_name.clone())
            .destination(source_proto)
            .identity("")
            .forward_to(message.get_incoming_conn())
            .session_type(self.session_type)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::MsgAck)
            .session_id(message.get_session_header().session_id)
            .message_id(message.get_id())
            .application_payload("", vec![]);

        if let Some(meta) = publish_meta {
            builder = builder.metadata_map(meta);
        }

        let mut output = SessionOutput::new();
        output.push_slim(builder.build_publish()?);
        Ok(output)
    }

    pub fn on_rtx_message(&mut self, message: Message) -> Result<SessionOutput, SessionError> {
        // in case we get the and RTX reply the session must be reliable
        let source_proto = message.get_slim_header().source.as_ref().unwrap();
        let encoded_source = source_proto.name.unwrap();
        let id = message.get_id();
        let in_conn = message.get_incoming_conn();

        debug!(
            %id, source = %source_proto,
            "received RTX reply");

        // remove the timer
        let key = PendingRtxKey {
            name: encoded_source,
            id,
        };
        if let Some(mut pending) = self.pending_rtxs.remove(&key) {
            pending.timer.stop();
        }

        // if rtx is not an error pass to on_publish_message
        // otherwise manage the message loss
        if message.get_error().is_none() {
            return self.on_publish_message(message);
        }

        // The buffer can be gone if the endpoint was removed (e.g. marked
        // offline) while this reply was in flight; that is not a bug in
        // itself, so do not abort processing over it -- see #2129.
        let Some(buffer) = self.buffer.get_mut(&encoded_source) else {
            debug!(%id, source = %source_proto, "ignore rtx error reply for a removed endpoint");
            return Ok(SessionOutput::new());
        };
        let recv_vec = buffer.on_lost_message(id);
        self.handle_recv_and_rtx_vectors(source_proto, in_conn, recv_vec, vec![])
    }

    fn handle_recv_and_rtx_vectors(
        &mut self,
        source: &ProtoName,
        in_conn: u64,
        recv_vec: Vec<Option<Message>>,
        rtx_vec: Vec<u32>,
    ) -> Result<SessionOutput, SessionError> {
        let mut output = SessionOutput::new();

        for recv in recv_vec {
            match recv {
                Some(r) => {
                    debug!(
                        id = %r.get_id(),
                        source = %r.get_source(),
                        "received message, send it to the app",
                    );
                    output.push_app(Ok(r));
                }
                None => {
                    debug!(
                        session_id = %self.session_id,
                        source = %source,
                        "lost message"
                    );
                    output.push_app(Err(SessionError::MessageLost(self.session_id)));
                }
            }
        }

        if !rtx_vec.is_empty() {
            let encoded = source.name.unwrap();

            for rtx_id in rtx_vec {
                debug!(
                    id = %rtx_id,
                    source = %source,
                    "send rtx");

                let rtx = Message::builder()
                    .source(self.local_name.clone())
                    .destination(source.clone())
                    .identity("")
                    .forward_to(in_conn)
                    .session_type(self.session_type)
                    .session_message_type(slim_datapath::api::ProtoSessionMessageType::RtxRequest)
                    .session_id(self.session_id)
                    .message_id(rtx_id)
                    .application_payload("", vec![])
                    .build_publish()?;

                // for each RTX start a timer
                debug!(id = %rtx_id,
                source = %source,"create rtx timer");

                let timer = self.timer_factory.as_ref().unwrap().create_and_start_timer(
                    rtx_id,
                    slim_datapath::api::ProtoSessionMessageType::RtxRequest,
                    Some(encoded),
                );

                let key = PendingRtxKey {
                    name: encoded,
                    id: rtx_id,
                };
                let val = PendingRtxVal {
                    timer,
                    message: rtx.clone(),
                };
                self.pending_rtxs.insert(key, val);

                // add message to output
                debug!(id = %rtx_id,
                source = %source, "send rtx request for message");
                output.push_slim(rtx);
            }
        }

        Ok(output)
    }

    pub fn on_timer_timeout(
        &mut self,
        id: u32,
        name: EncodedName,
    ) -> Result<SessionOutput, SessionError> {
        debug!(%id, "timeout for message");
        let key = PendingRtxKey { name, id };

        // The entry can already be gone if a different recovery path (an RTX
        // reply, remove_endpoint) resolved it first -- Timer::stop() only
        // cancels *future* firings, so an already in-flight notification
        // racing a different resolution is an ordinary occurrence, not a bug.
        // See #2133.
        let Some(pending) = self.pending_rtxs.get(&key) else {
            debug!(%id, "ignore stale timer timeout for an already-resolved rtx");
            return Ok(SessionOutput::new());
        };

        debug!(%id, "send rtx request again");
        let mut output = SessionOutput::new();
        output.push_slim(pending.message.clone());
        Ok(output)
    }

    pub fn on_timer_failure(
        &mut self,
        id: u32,
        name: EncodedName,
    ) -> Result<SessionOutput, SessionError> {
        debug!(
            %id,
            "timer failure for message, clear state",
        );
        let key = PendingRtxKey { name, id };

        // Same race as on_timer_timeout: the entry can already be gone if a
        // different recovery path resolved it first. See #2133.
        let Some(mut pending) = self.pending_rtxs.remove(&key) else {
            debug!(%id, "ignore stale timer failure for an already-resolved rtx");
            return Ok(SessionOutput::new());
        };

        // stop the timer and remove the name if no pending rtx left
        pending.timer.stop();

        // notify the application that the message was not delivered correctly
        let mut output = SessionOutput::new();
        output.push_app(Err(SessionError::receive_retry_failed(id)));

        // Mark the message as definitively lost in the reorder buffer too.
        // Without this, the buffer never learns the gap is unrecoverable, so
        // it keeps waiting on it forever and every later message already
        // buffered behind it is silently withheld.
        if let Some(buffer) = self.buffer.get_mut(&name) {
            let mut released = buffer.on_lost_message(id).into_iter();
            // The first (and, if there was still an earlier open gap, only)
            // entry is `id` itself, already reported above as
            // MessageReceiveRetryFailed -- skip it here so it is not also
            // reported as MessageLost, and forward whatever else the loss
            // unblocked.
            released.next();
            for recv in released {
                match recv {
                    Some(r) => output.push_app(Ok(r)),
                    None => output.push_app(Err(SessionError::MessageLost(self.session_id))),
                }
            }
        }
        Ok(output)
    }

    pub fn remove_endpoint(&mut self, endpoint: &ProtoName) {
        // remove the buffer related to an endpoint so that if it is added again
        // the messages will not be dropped as duplicated
        tracing::debug!(%endpoint, "remove endpoint on the receiver");
        let encoded = endpoint.name.unwrap();
        self.buffer.remove(&encoded);

        // Also stop and drop any pending RTX retries for this endpoint. Left
        // behind, a live timer can outlast the endpoint and later fire
        // against a *new* entry inserted at the same key if this endpoint
        // reconnects and hits the same message id again.
        self.pending_rtxs.retain(|key, val| {
            if key.name == encoded {
                val.timer.stop();
                false
            } else {
                true
            }
        });
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
    use crate::common::OutboundMessage;

    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    /// Helper to extract ToSlim messages from SessionOutput
    fn slim_messages(output: &SessionOutput) -> Vec<&Message> {
        output
            .messages
            .iter()
            .filter_map(|m| match m {
                OutboundMessage::ToSlim(msg) => Some(msg),
                _ => None,
            })
            .collect()
    }

    /// Helper to extract ToApp Ok messages from SessionOutput
    fn app_messages(output: &SessionOutput) -> Vec<&Message> {
        output
            .messages
            .iter()
            .filter_map(|m| match m {
                OutboundMessage::ToApp(Ok(msg)) => Some(msg),
                _ => None,
            })
            .collect()
    }

    /// Helper to extract ToApp Err messages from SessionOutput
    fn app_errors(output: &SessionOutput) -> Vec<&SessionError> {
        output
            .messages
            .iter()
            .filter_map(|m| match m {
                OutboundMessage::ToApp(Err(e)) => Some(e),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    #[traced_test]
    async fn test_receive_messages_1_and_2_sequentially() {
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        // Create and send message 1
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

        let output1 = receiver
            .on_message(message1)
            .expect("error sending message1");

        // Should have 1 ACK (to slim) + 1 app message
        let slims = slim_messages(&output1);
        assert_eq!(slims.len(), 1);
        assert_eq!(
            slims[0].get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MsgAck
        );
        assert_eq!(slims[0].get_session_header().get_message_id(), 1);

        let apps = app_messages(&output1);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].get_source(), remote_name);
        assert_eq!(apps[0].get_id(), 1);

        // Create and send message 2
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

        let output2 = receiver
            .on_message(message2)
            .expect("error sending message2");

        let slims = slim_messages(&output2);
        assert_eq!(slims.len(), 1);
        assert_eq!(slims[0].get_session_header().get_message_id(), 2);

        let apps = app_messages(&output2);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].get_id(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_loss_detection_with_rtx_timeout() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        // Send message 1
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

        let output1 = receiver
            .on_message(message1)
            .expect("error sending message1");
        assert_eq!(app_messages(&output1).len(), 1);
        assert_eq!(app_messages(&output1)[0].get_id(), 1);

        // Send message 3 (message 2 is missing) — triggers RTX
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

        let output3 = receiver
            .on_message(message3)
            .expect("error sending message3");

        // Should have ACK for msg 3 + RTX request for msg 2, no app messages yet (msg 3 buffered)
        let slims = slim_messages(&output3);
        assert_eq!(slims.len(), 2); // ACK + RTX
        let ack = slims.iter().find(|m| {
            m.get_session_message_type() == slim_datapath::api::ProtoSessionMessageType::MsgAck
        });
        assert!(ack.is_some());
        let rtx = slims.iter().find(|m| {
            m.get_session_message_type() == slim_datapath::api::ProtoSessionMessageType::RtxRequest
        });
        assert!(rtx.is_some());
        assert_eq!(rtx.unwrap().get_id(), 2);

        // No app messages (msg 3 is buffered waiting for msg 2)
        assert_eq!(app_messages(&output3).len(), 0);

        // Wait for the timer to fire
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerTimeout {
                message_id, name, ..
            } => {
                let output = receiver
                    .on_timer_timeout(message_id, name.unwrap())
                    .expect("error on timer timeout");
                let slims = slim_messages(&output);
                assert_eq!(slims.len(), 1);
                assert_eq!(slims[0].get_id(), 2);
            }
            _ => panic!("received unexpected message"),
        }

        // Wait for second timeout
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerTimeout {
                message_id, name, ..
            } => {
                let output = receiver
                    .on_timer_timeout(message_id, name.unwrap())
                    .expect("error on timer timeout");
                assert_eq!(slim_messages(&output).len(), 1);
            }
            _ => panic!("received unexpected message"),
        }

        // Wait for timer failure
        let signal_msg = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for timer failure")
            .expect("channel closed");

        match signal_msg {
            SessionMessage::TimerFailure {
                message_id, name, ..
            } => {
                let output = receiver
                    .on_timer_failure(message_id, name.unwrap())
                    .expect("error on timer failure");
                let errs = app_errors(&output);
                assert_eq!(errs.len(), 1);
                assert!(matches!(
                    errs[0],
                    SessionError::MessageReceiveRetryFailed { id: 2 }
                ));
                // Message 3, buffered behind the now-exhausted gap, must be
                // released in this same output rather than withheld forever.
                let delivered = app_messages(&output);
                assert_eq!(delivered.len(), 1);
                assert_eq!(delivered[0].get_id(), 3);
            }
            _ => panic!("received unexpected message"),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_reply_success() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        // Send message 1
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

        let output1 = receiver
            .on_message(message1)
            .expect("error sending message1");
        assert_eq!(app_messages(&output1).len(), 1);

        // Send message 3 (message 2 is missing)
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

        let _output3 = receiver
            .on_message(message3)
            .expect("error sending message3");

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

        let output_reply = receiver
            .on_message(rtx_reply)
            .expect("error sending rtx reply");

        // Should deliver messages 2 and 3 to app
        let apps = app_messages(&output_reply);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].get_id(), 2);
        assert_eq!(apps[1].get_id(), 3);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_reply_with_error() {
        let settings = TimerSettings::constant(Duration::from_millis(500)).with_max_retries(2);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        // Send message 1
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

        receiver
            .on_message(message1)
            .expect("error sending message1");

        // Send message 3 (message 2 is missing)
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

        receiver
            .on_message(message3)
            .expect("error sending message3");

        // Create RTX reply with error
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
        rtx_reply.get_slim_header_mut().set_error(Some(true));

        let output = receiver
            .on_message(rtx_reply)
            .expect("error sending rtx reply");

        // Should have an error (message lost) + message 3
        let apps = app_messages(&output);
        let errs = app_errors(&output);
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], SessionError::MessageLost(10)));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].get_id(), 3);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_senders_all_messages_delivered() {
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let remote1_name = ProtoName::from_strings(["org", "ns", "remote1"]);
        let remote2_name = ProtoName::from_strings(["org", "ns", "remote2"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        let mut all_app_msgs = Vec::new();
        let mut all_slim_msgs = Vec::new();

        // Send messages from remote1 and remote2
        for (remote, id) in [
            (&remote1_name, 1u32),
            (&remote2_name, 1u32),
            (&remote1_name, 2u32),
            (&remote2_name, 2u32),
        ] {
            let mut msg = Message::builder()
                .source(remote.clone())
                .destination(group_name.clone())
                .application_payload("payload", vec![id as u8])
                .build_publish()
                .unwrap();
            msg.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
            msg.get_session_header_mut().set_message_id(id);
            msg.get_session_header_mut().set_session_id(10);
            msg.get_slim_header_mut().set_incoming_conn(Some(1));

            let output = receiver.on_message(msg).expect("error sending message");
            for m in &output.messages {
                match m {
                    OutboundMessage::ToSlim(msg) => all_slim_msgs
                        .push((msg.get_dst(), msg.get_session_header().get_message_id())),
                    OutboundMessage::ToApp(Ok(msg)) => {
                        all_app_msgs.push((msg.get_source(), msg.get_id()))
                    }
                    _ => {}
                }
            }
        }

        // Verify all 4 app messages delivered
        assert_eq!(all_app_msgs.len(), 4);
        assert!(all_app_msgs.contains(&(remote1_name.clone(), 1)));
        assert!(all_app_msgs.contains(&(remote1_name.clone(), 2)));
        assert!(all_app_msgs.contains(&(remote2_name.clone(), 1)));
        assert!(all_app_msgs.contains(&(remote2_name.clone(), 2)));

        // Verify all 4 acks sent
        assert_eq!(all_slim_msgs.len(), 4);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_message_bypasses_reordering() {
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            Some(tx_signal),
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

        let output = receiver
            .on_message(message3)
            .expect("error sending message3");

        // Message should be delivered immediately (bypassing reordering)
        let apps = app_messages(&output);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].get_id(), 3);

        // ACK should have PUBLISH_TO metadata
        let slims = slim_messages(&output);
        assert_eq!(slims.len(), 1);
        assert!(slims[0].metadata.contains_key(PUBLISH_TO));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_in_p2p_session_receiver() {
        let settings = TimerSettings::constant(Duration::from_secs(10)).with_max_retries(1);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
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

        let output = receiver
            .on_message(message1)
            .expect("error sending message");

        // Message should be delivered
        assert_eq!(app_messages(&output).len(), 1);

        // Ack should NOT have PUBLISH_TO metadata (it was removed in P2P mode)
        let slims = slim_messages(&output);
        assert_eq!(slims.len(), 1);
        assert!(!slims[0].metadata.contains_key(PUBLISH_TO));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_publish_to_no_buffering_unreliable_mode() {
        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            None, // No timer settings = unreliable mode
            10,
            local_name.clone(),
            ProtoSessionType::Multicast,
            None,
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

        let output = receiver.on_message(message).expect("error sending message");

        // Message should be delivered to app
        assert_eq!(app_messages(&output).len(), 1);
        assert_eq!(app_messages(&output)[0].get_id(), 1);

        // No ack in unreliable mode
        assert_eq!(slim_messages(&output).len(), 0);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_remove_endpoint_stops_pending_retry() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(2);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        let publish = |id: u32| {
            let mut message = Message::builder()
                .source(remote_name.clone())
                .destination(local_name.clone())
                .application_payload("payload", vec![id as u8])
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
            message.get_session_header_mut().set_message_id(id);
            message.get_session_header_mut().set_session_id(10);
            message.get_slim_header_mut().set_incoming_conn(Some(1));
            message
        };

        receiver.on_message(publish(1)).unwrap();
        // gap at id 2 triggers an RTX request and a pending retry timer
        receiver.on_message(publish(3)).unwrap();
        assert!(!receiver.pending_rtxs.is_empty());

        receiver.remove_endpoint(&remote_name);

        // The pending retry, and its timer, must not survive endpoint removal.
        assert!(receiver.pending_rtxs.is_empty());
        assert!(
            timeout(Duration::from_millis(500), rx_signal.recv())
                .await
                .is_err(),
            "a stopped timer must not fire after its endpoint was removed"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_rtx_error_reply_tolerated_after_remove_endpoint() {
        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let mut receiver = SessionReceiver::new(
            Some(TimerSettings::constant(Duration::from_secs(60)).with_max_retries(2)),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        // The endpoint is removed (e.g. marked offline) before its error RTX
        // reply arrives -- the buffer entry is gone, but this must not abort
        // processing.
        receiver.remove_endpoint(&remote_name);

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
        rtx_reply.get_slim_header_mut().set_error(Some(true));

        let output = receiver
            .on_rtx_message(rtx_reply)
            .expect("must not error for a removed endpoint");
        assert!(app_messages(&output).is_empty());
        assert!(app_errors(&output).is_empty());
        assert!(slim_messages(&output).is_empty());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stale_timer_notification_after_retry_exhaustion_is_tolerated() {
        // Minimal case found by the proptest_stateful module (#2133): a
        // TimerFailure that already resolved a gap must not make a later,
        // racing TimerTimeout for the same id hard-error -- Timer::stop()
        // only cancels *future* firings, so the in-flight notification is an
        // ordinary occurrence, not a bug.
        let settings = TimerSettings::constant(Duration::from_secs(60)).with_max_retries(2);
        let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(10);

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let encoded_remote = remote_name.name.unwrap();

        let mut receiver = SessionReceiver::new(
            Some(settings),
            10,
            local_name.clone(),
            ProtoSessionType::PointToPoint,
            Some(tx_signal),
        );

        let publish = |id: u32| {
            let mut message = Message::builder()
                .source(remote_name.clone())
                .destination(local_name.clone())
                .application_payload("payload", vec![id as u8])
                .build_publish()
                .unwrap();
            message.set_session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg);
            message.get_session_header_mut().set_message_id(id);
            message.get_session_header_mut().set_session_id(10);
            message.get_slim_header_mut().set_incoming_conn(Some(1));
            message
        };

        receiver.on_message(publish(1)).unwrap();
        // gap at id 2 triggers an RTX request and a pending retry timer
        receiver.on_message(publish(3)).unwrap();
        assert!(!receiver.pending_rtxs.is_empty());

        // Retry exhaustion resolves and removes the pending entry for id 2.
        receiver
            .on_timer_failure(2, encoded_remote)
            .expect("timer failure must succeed while the entry is still live");
        assert!(receiver.pending_rtxs.is_empty());

        // A duplicate/racing timeout notification for the same id arrives
        // after the failure already resolved it.
        let output = receiver
            .on_timer_timeout(2, encoded_remote)
            .expect("a stale timer timeout must not hard-error");
        assert!(slim_messages(&output).is_empty());
        assert!(app_messages(&output).is_empty());
        assert!(app_errors(&output).is_empty());
    }
}

/// Property-based (stateful) testing over `SessionReceiver`'s own operation
/// sequence, per #2133 — sequencing/state bugs like #2129/#2130 involve only
/// well-formed messages (invisible to decode-level fuzzing) and don't panic
/// (cargo-fuzz's default signal), so this drives the receiver's public API
/// through generated call sequences and checks invariants directly on its
/// state instead.
#[cfg(test)]
mod proptest_stateful {
    use std::collections::HashSet;
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;

    const NUM_ENDPOINTS: u8 = 2;
    const MAX_ID: u32 = 6;

    #[derive(Debug, Clone, Copy)]
    enum PublishKind {
        Msg,
        RtxReplyOk,
        RtxReplyErr,
    }

    #[derive(Debug, Clone, Copy)]
    enum Op {
        Publish {
            endpoint: u8,
            id: u32,
            kind: PublishKind,
        },
        TimerTimeout {
            history_idx: usize,
        },
        TimerFailure {
            history_idx: usize,
        },
        RemoveEndpoint {
            endpoint: u8,
        },
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (0..NUM_ENDPOINTS, 1..=MAX_ID, prop_oneof![
                Just(PublishKind::Msg),
                Just(PublishKind::RtxReplyOk),
                Just(PublishKind::RtxReplyErr),
            ])
                .prop_map(|(endpoint, id, kind)| Op::Publish { endpoint, id, kind }),
            1 => (0usize..64).prop_map(|history_idx| Op::TimerTimeout { history_idx }),
            1 => (0usize..64).prop_map(|history_idx| Op::TimerFailure { history_idx }),
            1 => (0..NUM_ENDPOINTS).prop_map(|endpoint| Op::RemoveEndpoint { endpoint }),
        ]
    }

    fn endpoint_name(idx: u8) -> ProtoName {
        ProtoName::from_strings(["org", "ns", &format!("remote{idx}")])
    }

    /// Applies one generated `Op` to `receiver`, returning the underlying
    /// `SessionReceiver` call's result — `None` for `RemoveEndpoint` (no
    /// `Result` to check) and for a `TimerTimeout`/`TimerFailure` whose
    /// `history_idx` has nothing to index yet.
    fn apply(
        receiver: &mut SessionReceiver,
        local_name: &ProtoName,
        history: &mut Vec<(EncodedName, u32)>,
        op: Op,
    ) -> Option<Result<SessionOutput, SessionError>> {
        let before: HashSet<(EncodedName, u32)> = receiver
            .pending_rtxs
            .keys()
            .map(|k| (k.name, k.id))
            .collect();

        let result = match op {
            Op::Publish { endpoint, id, kind } => {
                let remote = endpoint_name(endpoint);
                let mut message = Message::builder()
                    .source(remote.clone())
                    .destination(local_name.clone())
                    .application_payload("payload", vec![id as u8])
                    .build_publish()
                    .unwrap();
                message.get_session_header_mut().set_message_id(id);
                message.get_session_header_mut().set_session_id(10);
                message.get_slim_header_mut().set_incoming_conn(Some(1));
                match kind {
                    PublishKind::Msg => {
                        message.set_session_message_type(
                            slim_datapath::api::ProtoSessionMessageType::Msg,
                        );
                    }
                    PublishKind::RtxReplyOk => {
                        message.set_session_message_type(
                            slim_datapath::api::ProtoSessionMessageType::RtxReply,
                        );
                    }
                    PublishKind::RtxReplyErr => {
                        message.set_session_message_type(
                            slim_datapath::api::ProtoSessionMessageType::RtxReply,
                        );
                        message.get_slim_header_mut().set_error(Some(true));
                    }
                }
                Some(receiver.on_message(message))
            }
            Op::TimerTimeout { history_idx } => {
                if history.is_empty() {
                    None
                } else {
                    let (name, id) = history[history_idx % history.len()];
                    Some(receiver.on_timer_timeout(id, name))
                }
            }
            Op::TimerFailure { history_idx } => {
                if history.is_empty() {
                    None
                } else {
                    let (name, id) = history[history_idx % history.len()];
                    Some(receiver.on_timer_failure(id, name))
                }
            }
            Op::RemoveEndpoint { endpoint } => {
                receiver.remove_endpoint(&endpoint_name(endpoint));
                None
            }
        };

        // Every (name, id) that just became a pending RTX is a key this
        // receiver itself created — a later timer op is allowed to target it
        // even after it's since been resolved another way (that's exactly
        // the stale-notification race being tested for), since a real timer
        // notification could never reference anything else.
        let after = receiver.pending_rtxs.keys().map(|k| (k.name, k.id));
        history.extend(after.filter(|k| !before.contains(k)));

        result
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// Two invariants over `SessionReceiver`'s state, checked after every
        /// generated operation:
        ///
        /// - `pending_rtxs` never holds an entry for a name absent from
        ///   `buffer` — direct regression coverage for #2129's
        ///   `remove_endpoint` gap.
        /// - A timer notification for a key this receiver itself created
        ///   never hard-errors, even when a different recovery path (an RTX
        ///   reply, `remove_endpoint`) already resolved it first —
        ///   `Timer::stop()` only cancels *future* firings, so an
        ///   already-in-flight notification racing a different resolution is
        ///   an ordinary occurrence, not caller misuse.
        #[test]
        fn receiver_never_orphans_pending_rtx_or_hard_errors_on_stale_timer(
            ops in proptest::collection::vec(op_strategy(), 1..40),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let settings = TimerSettings::constant(Duration::from_secs(30)).with_max_retries(5);
                let (tx_signal, _rx_signal) = tokio::sync::mpsc::channel(64);
                let local_name = ProtoName::from_strings(["org", "ns", "local"]);

                let mut receiver = SessionReceiver::new(
                    Some(settings),
                    10,
                    local_name.clone(),
                    ProtoSessionType::PointToPoint,
                    Some(tx_signal),
                );
                let mut history: Vec<(EncodedName, u32)> = Vec::new();

                for op in ops {
                    let result = apply(&mut receiver, &local_name, &mut history, op);

                    if let Some(res) = &result {
                        prop_assert!(
                            res.is_ok(),
                            "op {op:?} returned {res:?}, which would abort a draining processing loop"
                        );
                    }

                    for key in receiver.pending_rtxs.keys() {
                        prop_assert!(
                            receiver.buffer.contains_key(&key.name),
                            "pending_rtxs has an entry for id {} with no matching buffer entry after op {op:?}",
                            key.id,
                        );
                    }
                }

                Ok(())
            })?;
        }
    }
}
