// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use display_error_chain::ErrorChainExt;
use slim_datapath::api::{
    CommandPayload, ProtoMessage as Message, ProtoName, ProtoSessionMessageType, ProtoSessionType,
};
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::{
    SessionError,
    common::{SessionMessage, SessionOutput},
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
};

/// Heartbeat interval.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// Maximum number of consecutive missed heartbeats before a participant is considered offline.
const MAX_MISSED_HEARTBEATS: u32 = 3;

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
    missing_replies: HashSet<ProtoName>,

    /// Message to resend in case of timeout
    message: Message,

    /// the timer
    timer: Timer,
}

struct HeartbeatState {
    /// Tracks the number of consecutive missed heartbeats per participant.
    /// Reset to 0 when a heartbeat is received from that participant.
    /// Incremented on each heartbeat timer tick.
    missed_heartbeats: HashMap<ProtoName, u32>,

    /// When true, skip sending the next heartbeat because we already
    /// sent other traffic during this interval. Reset to false on each tick.
    postpone_heartbeat: bool,

    /// The periodic heartbeat timer
    heartbeat_timer: Timer,

    /// Timer factory for creating heartbeat timers
    #[allow(dead_code)]
    heartbeat_timer_factory: TimerFactory,
}

pub struct ControllerSender {
    /// timer factory to crate timers for acks
    timer_factory: TimerFactory,

    /// local name to be removed in the missing replies set
    local_name: ProtoName,

    /// group name is set on the first join request message
    /// in p2p session is equal to the remote name of the join request
    /// while in multicast session is specified in the payload
    group_name: Option<ProtoName>,

    /// session type
    session_type: ProtoSessionType,

    /// session id
    session_id: u32,

    /// list of pending replies for each control message
    pending_replies: HashMap<u32, PendingReply>,

    /// heartbeat state
    /// by default is None, start only if a duration is set
    heartbeat_state: Option<HeartbeatState>,

    /// set to true if the participant is an initiator
    #[allow(dead_code)]
    initiator: bool,

    /// group list
    /// list of participants to the group
    group_list: HashSet<ProtoName>,

    /// send message to the session controller
    tx_session: Sender<SessionMessage>,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ControllerSenderDrainStatus,
}

impl ControllerSender {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timer_settings: TimerSettings,
        local_name: ProtoName,
        session_type: ProtoSessionType,
        session_id: u32,
        heartbeat_interval: Option<Duration>,
        initiator: bool,
        tx_signals: Sender<SessionMessage>,
    ) -> Self {
        let mut list = HashSet::new();
        list.insert(local_name.clone());

        let heartbeat_state = if let Some(interval) = heartbeat_interval {
            let settings =
                TimerSettings::new(interval, None, None, crate::timer::TimerType::Constant);
            let heartbeat_timer_factory = TimerFactory::new(settings, tx_signals.clone());
            let heartbeat_timer = heartbeat_timer_factory.create_and_start_timer(
                rand::random::<u32>(),
                slim_datapath::api::ProtoSessionMessageType::Heartbeat,
                None,
            );
            Some(HeartbeatState {
                missed_heartbeats: HashMap::new(),
                postpone_heartbeat: false,
                heartbeat_timer,
                heartbeat_timer_factory,
            })
        } else {
            None
        };

        ControllerSender {
            timer_factory: TimerFactory::new(timer_settings, tx_signals.clone()),
            local_name,
            group_name: None,
            session_type,
            session_id,
            pending_replies: HashMap::new(),
            heartbeat_state,
            initiator,
            group_list: list,
            tx_session: tx_signals,
            draining_state: ControllerSenderDrainStatus::NotDraining,
        }
    }

    pub fn add_participant(&mut self, name: &ProtoName) {
        debug!(
            participant = %name,
            "adding participant to group on controller sender"
        );
        self.group_list.insert(name.clone());

        // Start tracking heartbeats for this participant
        if let Some(hs) = self.heartbeat_state.as_mut() {
            hs.missed_heartbeats.insert(name.clone(), 0);
        }
    }

    pub fn remove_participant(&mut self, name: &ProtoName) {
        debug!(
            participant = %name,
            "removing participant from group on controller sender"
        );
        self.group_list.remove(name);

        // remove also the heartbeat tracking state if present
        if let Some(hs) = self.heartbeat_state.as_mut() {
            hs.missed_heartbeats.remove(name);
        }
    }

    pub fn stop_heartbeat(&mut self) {
        if let Some(hs) = self.heartbeat_state.as_mut() {
            debug!("stop heartbeat timer");
            hs.heartbeat_timer.stop();
            hs.missed_heartbeats.clear();
        }
    }

    pub fn restart_heartbeat(&mut self) {
        if let Some(hs) = self.heartbeat_state.as_mut() {
            debug!("restart heartbeat timer");
            hs.heartbeat_timer = hs.heartbeat_timer_factory.create_and_start_timer(
                rand::random::<u32>(),
                slim_datapath::api::ProtoSessionMessageType::Heartbeat,
                None,
            );
            // Re-initialize missed heartbeat tracking for all group members
            for name in &self.group_list {
                if name != &self.local_name {
                    hs.missed_heartbeats.insert(name.clone(), 0);
                }
            }
        }
    }

    // helper function to update local state based on the message type received
    fn update_local_state(&mut self, message: &Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::JoinRequest
                if self.group_name.is_none() =>
            {
                // setup the group name if not set yet
                if self.session_type == ProtoSessionType::PointToPoint {
                    // in p2p session the group name is equal to the remote name
                    // in the join request message. Data and control messages
                    // are distributed using the same name.
                    debug!(
                        destination = %message.get_dst(),
                        "update group name on join request message for p2p session",
                    );
                    self.group_name = Some(message.get_dst());
                } else {
                    // in multicast session the group name is specified in the
                    // payload of the message, here we use only the control
                    // channel name that must be set
                    let group_name = message
                        .extract_join_request()?
                        .control
                        .as_ref()
                        .ok_or(SessionError::MissingGroupNameInJoinRequest)?
                        .clone();
                    debug!(
                        destination = %group_name,
                        "update group name on join request message for multicast session",
                    );
                    self.group_name = Some(group_name);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn on_message(&mut self, message: &Message) -> Result<SessionOutput, SessionError> {
        if self.draining_state == ControllerSenderDrainStatus::Completed {
            return Err(SessionError::SessionDrainingDrop);
        }
        let mut output = SessionOutput::new();

        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest
            | slim_datapath::api::ProtoSessionMessageType::JoinRequest
            | slim_datapath::api::ProtoSessionMessageType::LeaveRequest
            | slim_datapath::api::ProtoSessionMessageType::GroupWelcome => {
                if self.draining_state == ControllerSenderDrainStatus::Initiated {
                    // draining period started; reject new messages
                    return Err(SessionError::SessionDrainingDrop);
                }

                // create the set of missing replies
                let mut missing_replies = HashSet::new();
                let mut name = message.get_dst();
                if message.get_session_message_type()
                    == slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest
                {
                    // the discovery request should be sent to an unknown destination id.
                    // if the id is present remove it for consistency on the ack return.
                    // this affects only the ack registration and not the forwarding behaviour.
                    name.reset_id();
                }
                missing_replies.insert(name);

                // update local state
                self.update_local_state(message)?;

                // send the message and setup the required timers
                output.extend(self.on_send_message(message, missing_replies)?);
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
            slim_datapath::api::ProtoSessionMessageType::Heartbeat => {
                self.on_heartbeat_received(message);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupAdd => {
                // compute the list of participants that needs to send an ack
                // remove the local name as we are not waiting for any reply from the local name
                // remove also the new participant as it will not receive the message
                let mut missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                // remove the new participant also from the missing replies
                let payload = message.extract_group_add()?;

                let to_remove = payload
                    .new_participant
                    .as_ref()
                    .ok_or(SessionError::MissingNewParticipantInGroupAdd)?
                    .get_name()?
                    .clone();

                missing_replies.remove(&to_remove);

                debug!(
                    "send group add with message id {}, expected acks from {:?}",
                    message.get_id(),
                    missing_replies
                );

                output.extend(self.on_send_message(message, missing_replies)?);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupRemove => {
                // compute the list of participants that needs to send an ack
                // the participant that we are removing will get the update
                // so we can use the group list as is, removing only the local name
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                output.extend(self.on_send_message(message, missing_replies)?);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupClose => {
                // compute the list of participants that needs to send an ack
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                output.extend(self.on_send_message(message, missing_replies)?);
            }
            slim_datapath::api::ProtoSessionMessageType::UpdateParticipantState => {
                // compute the list of participants that needs to send an ack
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                output.extend(self.on_send_message(message, missing_replies)?);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupProposal => todo!(),
            _ => {
                debug!("unexpected message type");
            }
        }

        Ok(output)
    }

    fn on_send_message(
        &mut self,
        message: &Message,
        missing_replies: HashSet<ProtoName>,
    ) -> Result<SessionOutput, SessionError> {
        let id = message.get_id();

        debug!(
            %id, ?missing_replies,
            "create a new timer for message, waiting responses",
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

        // Sending a message to the group channel counts as activity
        if self.group_name.as_ref() == Some(&message.get_dst()) {
            self.notify_sent_activity();
        }

        let mut output = SessionOutput::new();
        output.push_slim(message.clone());
        Ok(output)
    }

    fn on_reply_message(&mut self, message: &Message) {
        let id = message.get_id();
        debug!(
            %id,
            source = %message.get_source(),
            "receive reply for message",
        );

        let mut delete = false;
        if let Some(pending) = self.pending_replies.get_mut(&id) {
            debug!(%id, "try to remove from pending acks");
            let mut name = message.get_source();
            if message.get_session_message_type()
                == slim_datapath::api::ProtoSessionMessageType::DiscoveryReply
            {
                name.reset_id();
            }
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

    fn on_heartbeat_received(&mut self, message: &Message) {
        let source = message.get_source();
        debug!(from = %source, "received heartbeat");
        if let Some(heartbeat_state) = &mut self.heartbeat_state {
            // Reset the missed counter only if this participant is already tracked
            if heartbeat_state.missed_heartbeats.contains_key(&source) {
                heartbeat_state.missed_heartbeats.insert(source, 0);
            }
        }
    }

    /// Notify that we sent traffic on the channel, so the next heartbeat can be skipped.
    pub fn notify_sent_activity(&mut self) {
        if let Some(hs) = self.heartbeat_state.as_mut() {
            hs.postpone_heartbeat = true;
        }
    }

    /// Notify that we received traffic from a remote participant,
    /// which counts as proof of liveness (same as receiving a heartbeat).
    pub fn notify_received_activity(&mut self, message: &Message) {
        self.on_heartbeat_received(message);
    }

    pub fn is_still_pending(&self, message_id: u32) -> bool {
        self.pending_replies.contains_key(&message_id)
    }

    pub(crate) fn on_timer_timeout(
        &mut self,
        id: u32,
        msg_type: ProtoSessionMessageType,
        mls_epoch: Option<u64>,
    ) -> Result<SessionOutput, SessionError> {
        debug!(%id, ?msg_type, "timeout for message");

        // check if the timeout is related to a heartbeat
        if self.heartbeat_state.is_some()
            && msg_type == ProtoSessionMessageType::Heartbeat
            && let Some(epoch) = mls_epoch
        {
            return self.handle_heartbeat_timeout(id, epoch);
        }

        // the timer is not related to a heartbeat, resend the message if possible
        if let Some(pending) = self.pending_replies.get(&id) {
            let mut output = SessionOutput::new();
            output.push_slim(pending.message.clone());
            return Ok(output);
        };

        Err(SessionError::TimerNotFound(id))
    }

    fn handle_heartbeat_timeout(
        &mut self,
        id: u32,
        epoch: u64,
    ) -> Result<SessionOutput, SessionError> {
        let is_heartbeat_timer = self
            .heartbeat_state
            .as_ref()
            .map(|hs| hs.heartbeat_timer.get_id() == id)
            .unwrap_or(false);

        if !is_heartbeat_timer {
            return Ok(SessionOutput::new());
        }

        debug!("heartbeat timer tick, check group state and send heartbeat");

        // Increment missed counter for all tracked participants and detect offline
        if let Some(heartbeat_state) = self.heartbeat_state.as_mut() {
            for (_, count) in heartbeat_state.missed_heartbeats.iter_mut() {
                *count += 1;
            }

            // Detect offline participants
            let offline: Vec<ProtoName> = heartbeat_state
                .missed_heartbeats
                .iter()
                .filter(|(_, count)| **count >= MAX_MISSED_HEARTBEATS)
                .map(|(name, _)| name.clone())
                .collect();

            for name in &offline {
                debug!(participant = %name, "participant detected offline (missed heartbeats)");
                heartbeat_state.missed_heartbeats.remove(name);
                if let Err(e) = self
                    .tx_session
                    .try_send(SessionMessage::ParticipantDisconnected {
                        name: Some(name.clone()),
                    })
                {
                    debug!(error = %e.chain(), "failed to send participant disconnected message");
                }
            }
        }

        // Send our heartbeat broadcast only if we didn't already send traffic
        let mut output = SessionOutput::new();
        let should_send = self
            .heartbeat_state
            .as_ref()
            .map(|hs| !hs.postpone_heartbeat)
            .unwrap_or(false);

        if should_send
            && self.group_list.len() > 1
            && let Some(group_name) = &self.group_name
        {
            let heartbeat_id = rand::random::<u32>();
            let mut builder = Message::builder()
                .source(self.local_name.clone())
                .destination(group_name.clone())
                .identity("")
                .session_type(self.session_type)
                .session_message_type(ProtoSessionMessageType::Heartbeat)
                .session_id(self.session_id)
                .message_id(heartbeat_id)
                .payload(CommandPayload::builder().heartbeat(epoch).as_content());

            if self.session_type == ProtoSessionType::Multicast {
                builder = builder.fanout(256);
            }

            let heartbeat_msg = builder.build_publish()?;
            debug!(id = %heartbeat_id, "send heartbeat");
            output.push_slim(heartbeat_msg);
        }

        // Always reset the flag for the next interval
        if let Some(hs) = self.heartbeat_state.as_mut() {
            hs.postpone_heartbeat = false;
        }

        Ok(output)
    }

    pub fn on_failure(&mut self, id: u32, _msg_type: ProtoSessionMessageType) -> Vec<ProtoName> {
        if let Some(gt) = self.pending_replies.get_mut(&id) {
            gt.timer.stop();
        }

        self.pending_replies
            .remove(&id)
            .map(|p| p.missing_replies.into_iter().collect::<Vec<_>>())
            .unwrap_or_default()
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
    use super::*;
    use crate::common::{OutboundMessage, SessionOutput};
    use slim_datapath::api::{
        CommandPayload, Participant, ParticipantSettings, ProtoSessionMessageType, ProtoSessionType,
    };
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    fn single_slim_message(output: SessionOutput) -> Message {
        let mut messages = output
            .messages
            .into_iter()
            .map(|message| match message {
                OutboundMessage::ToSlim(message) => message,
                OutboundMessage::ToApp(_) => panic!("Expected ToSlim message"),
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1, "Expected exactly one outbound message");
        messages.pop().unwrap()
    }

    fn assert_no_messages(output: SessionOutput) {
        assert!(output.messages.is_empty(), "Expected no outbound messages");
    }

    async fn expect_timeout(
        rx_signal: &mut tokio::sync::mpsc::Receiver<SessionMessage>,
        wait: Duration,
    ) -> (u32, ProtoSessionMessageType) {
        let timeout_msg = timeout(wait, rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => (message_id, message_type),
            other => panic!("Expected TimerTimeout message, got {:?}", other),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_discovery_request() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().discovery_request().as_content())
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&request).expect("error sending message"));
        assert_eq!(sent, request);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::DiscoveryRequest, None)
                .expect("error re-sending the request"),
        );
        assert_eq!(resent, request);

        let reply = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryReply)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().discovery_reply().as_content())
            .build_publish()
            .unwrap();

        assert_no_messages(sender.on_message(&reply).expect("error sending reply"));
        assert!(
            timeout(Duration::from_millis(400), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_join_request() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let channel_data = ProtoName::from_strings(["org", "ns", "channel"]).with_id(1);
        let channel_control = ProtoName::from_strings(["org", "ns", "channel"]).with_id(2);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        None,
                        None,
                        Some(channel_data.clone()),
                        Some(channel_control.clone()),
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&request).expect("error sending message"));
        assert_eq!(sent, request);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::JoinRequest);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::JoinRequest, None)
                .expect("error re-sending the request"),
        );
        assert_eq!(resent, request);

        let reply = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .join_reply(
                        None,
                        Participant::new(remote.clone(), ParticipantSettings::bidirectional()),
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        assert_no_messages(sender.on_message(&reply).expect("error sending reply"));
        assert!(
            timeout(Duration::from_millis(400), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_leave_request() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().leave_request().as_content())
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&request).expect("error sending message"));
        assert_eq!(sent, request);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::LeaveRequest);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::LeaveRequest, None)
                .expect("error re-sending the request"),
        );
        assert_eq!(resent, request);

        let reply = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveReply)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().leave_reply().as_content())
            .build_publish()
            .unwrap();

        assert_no_messages(sender.on_message(&reply).expect("error sending reply"));
        assert!(
            timeout(Duration::from_millis(400), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_welcome() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source_name = ProtoName::from_strings(["org", "ns", "source"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);
        let source = Participant::new(source_name.clone(), ParticipantSettings::bidirectional());
        let remote = Participant::new(remote_name.clone(), ParticipantSettings::bidirectional());
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source_name.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let welcome = Message::builder()
            .source(source_name.clone())
            .destination(remote_name.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![remote.clone(), source.clone()], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&welcome).expect("error sending message"));
        assert_eq!(sent, welcome);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::GroupWelcome);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::GroupWelcome, None)
                .expect("error re-sending the welcome"),
        );
        assert_eq!(resent, welcome);

        let ack = Message::builder()
            .source(remote_name.clone())
            .destination(source_name.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();

        assert_no_messages(sender.on_message(&ack).expect("error sending ack"));
        assert!(
            timeout(Duration::from_millis(400), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_add_message() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let group = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let participant2 = ProtoName::from_strings(["org", "ns", "participant2"]);
        sender.group_list.insert(participant2.clone());

        let participant1 = ProtoName::from_strings(["org", "ns", "participant1"]);
        let update = Message::builder()
            .source(source.clone())
            .destination(group.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAdd)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .group_add(
                        Participant::new(
                            participant1.clone(),
                            ParticipantSettings::bidirectional(),
                        ),
                        vec![
                            Participant::new(
                                participant1.clone(),
                                ParticipantSettings::bidirectional(),
                            ),
                            Participant::new(
                                participant2.clone(),
                                ParticipantSettings::bidirectional(),
                            ),
                            Participant::new(source.clone(), ParticipantSettings::bidirectional()),
                        ],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&update).expect("error sending message"));
        assert_eq!(sent, update);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd, None)
                .expect("error re-sending the add"),
        );
        assert_eq!(resent, update);

        let ack1 = Message::builder()
            .source(participant1.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();
        assert_no_messages(sender.on_message(&ack1).expect("error sending ack"));
        assert!(sender.is_still_pending(1));

        let ack2 = Message::builder()
            .source(participant2.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();
        assert_no_messages(sender.on_message(&ack2).expect("error sending ack"));
        assert!(!sender.is_still_pending(1));
        assert!(
            timeout(Duration::from_millis(100), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_on_group_update_duplicate_acks() {
        let settings = TimerSettings::constant(Duration::from_millis(200)).with_max_retries(3);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx_signal,
        );

        let participant2 = ProtoName::from_strings(["org", "ns", "participant2"]);
        sender.group_list.insert(participant2.clone());

        let participant1 = ProtoName::from_strings(["org", "ns", "participant1"]);
        let update = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAdd)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .group_add(
                        Participant::new(
                            participant1.clone(),
                            ParticipantSettings::bidirectional(),
                        ),
                        vec![
                            Participant::new(
                                participant1.clone(),
                                ParticipantSettings::bidirectional(),
                            ),
                            Participant::new(
                                participant2.clone(),
                                ParticipantSettings::bidirectional(),
                            ),
                            Participant::new(source.clone(), ParticipantSettings::bidirectional()),
                        ],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sent = single_slim_message(sender.on_message(&update).expect("error sending message"));
        assert_eq!(sent, update);

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);

        let resent = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd, None)
                .expect("error re-sending the add"),
        );
        assert_eq!(resent, update);

        let ack1 = Message::builder()
            .source(participant1.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();
        assert_no_messages(sender.on_message(&ack1).expect("error sending ack"));
        assert!(sender.is_still_pending(1));

        assert_no_messages(
            sender
                .on_message(&ack1)
                .expect("error sending duplicate ack"),
        );
        assert!(sender.is_still_pending(1));

        let (message_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
        assert_eq!(message_id, 1);
        assert_eq!(message_type, ProtoSessionMessageType::GroupAdd);

        let retransmitted = single_slim_message(
            sender
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd, None)
                .expect("error re-sending the add"),
        );
        assert_eq!(retransmitted, update);

        let ack2 = Message::builder()
            .source(participant2.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(session_id)
            .message_id(1)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();
        assert_no_messages(sender.on_message(&ack2).expect("error sending ack"));
        assert!(!sender.is_still_pending(1));
        assert!(
            timeout(Duration::from_millis(100), rx_signal.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_sends_on_timer_tick() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.group_list.insert(participant.clone());
        sender.group_name = Some(group_name.clone());

        // Wait for the heartbeat timer to fire
        let (timer_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);

        let output = sender
            .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");
        let heartbeat_msg = single_slim_message(output);
        assert_eq!(
            heartbeat_msg.get_session_message_type(),
            ProtoSessionMessageType::Heartbeat
        );
        assert_eq!(heartbeat_msg.get_dst(), group_name);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_received_resets_counter() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.add_participant(&participant);
        sender.group_name = Some(group_name.clone());

        // Simulate receiving a heartbeat from the participant
        let heartbeat_from_participant = Message::builder()
            .source(participant.clone())
            .destination(group_name.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Heartbeat)
            .session_id(session_id)
            .message_id(rand::random::<u32>())
            .payload(CommandPayload::builder().heartbeat(50).as_content())
            .build_publish()
            .unwrap();

        sender
            .on_message(&heartbeat_from_participant)
            .expect("error processing heartbeat");

        // Verify the missed counter is 0
        if let Some(hs) = &sender.heartbeat_state {
            assert_eq!(hs.missed_heartbeats.get(&participant).copied(), Some(0));
        } else {
            panic!("Heartbeat state should be initialized");
        }

        // Wait for timer tick — should increment to 1 (not trigger disconnect)
        let (timer_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);
        sender
            .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");

        if let Some(hs) = &sender.heartbeat_state {
            assert_eq!(hs.missed_heartbeats.get(&participant).copied(), Some(1));
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_detects_offline_participant() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(200);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.add_participant(&participant);
        sender.group_name = Some(group_name.clone());
        let heartbeat_from_participant = Message::builder()
            .source(participant.clone())
            .destination(group_name.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Heartbeat)
            .session_id(session_id)
            .message_id(rand::random::<u32>())
            .payload(CommandPayload::builder().heartbeat(50).as_content())
            .build_publish()
            .unwrap();
        sender
            .on_message(&heartbeat_from_participant)
            .expect("error processing heartbeat");

        // Now let MAX_MISSED_HEARTBEATS timer ticks pass without any heartbeat
        for _ in 0..MAX_MISSED_HEARTBEATS {
            let (timer_id, message_type) =
                expect_timeout(&mut rx_signal, Duration::from_millis(300)).await;
            assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);
            sender
                .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
                .expect("error handling heartbeat timeout");
        }

        // Should have received a ParticipantDisconnected message
        match rx_signal.try_recv() {
            Ok(SessionMessage::ParticipantDisconnected { name }) => {
                assert_eq!(name, Some(participant.clone()));
            }
            Ok(other) => panic!("Expected ParticipantDisconnected message, got {:?}", other),
            Err(e) => panic!(
                "Expected ParticipantDisconnected message, channel error: {:?}",
                e
            ),
        }

        // Participant should be removed from tracking
        if let Some(hs) = &sender.heartbeat_state {
            assert!(!hs.missed_heartbeats.contains_key(&participant));
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_p2p_destination() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::PointToPoint,
            session_id,
            Some(heartbeat_interval),
            true,
            tx_signal,
        );

        sender.group_list.insert(remote.clone());
        sender.group_name = Some(remote.clone());

        let (timer_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);

        let output = sender
            .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");
        let heartbeat_msg = single_slim_message(output);
        assert_eq!(
            heartbeat_msg.get_session_message_type(),
            ProtoSessionMessageType::Heartbeat
        );
        assert_eq!(heartbeat_msg.get_dst(), remote);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_no_send_when_alone() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.group_name = Some(ProtoName::from_strings(["org", "ns", "group"]));

        // Wait for timer tick — should produce no outbound messages (alone in group)
        let (timer_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);

        let output = sender
            .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");
        assert_no_messages(output);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_heartbeat_suppressed_when_sent_activity() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.add_participant(&participant);
        sender.group_name = Some(group_name.clone());

        // Mark that we sent activity on the channel
        sender.notify_sent_activity();

        // Wait for heartbeat timer tick
        let (timer_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);

        // Heartbeat should be suppressed because we sent activity
        let output = sender
            .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");
        assert_no_messages(output);

        // Next tick should send heartbeat since postpone_heartbeat was reset
        let (timer_id2, message_type2) =
            expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
        assert_eq!(message_type2, ProtoSessionMessageType::Heartbeat);

        let output2 = sender
            .on_timer_timeout(timer_id2, ProtoSessionMessageType::Heartbeat, Some(50))
            .expect("error handling heartbeat timeout");
        let msg = single_slim_message(output2);
        assert_eq!(
            msg.get_session_message_type(),
            ProtoSessionMessageType::Heartbeat
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_received_activity_resets_counter() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let heartbeat_interval = Duration::from_millis(500);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(heartbeat_interval),
            false,
            tx_signal,
        );

        sender.add_participant(&participant);
        sender.group_name = Some(group_name.clone());

        // Let 2 timer ticks pass (missed count goes to 2)
        for _ in 0..2 {
            let (timer_id, message_type) =
                expect_timeout(&mut rx_signal, Duration::from_millis(600)).await;
            assert_eq!(message_type, ProtoSessionMessageType::Heartbeat);
            sender
                .on_timer_timeout(timer_id, ProtoSessionMessageType::Heartbeat, Some(50))
                .expect("error handling heartbeat timeout");
        }

        // Verify counter is at 2
        if let Some(hs) = &sender.heartbeat_state {
            assert_eq!(hs.missed_heartbeats.get(&participant).copied(), Some(2));
        }

        // Simulate receiving a non-heartbeat message (e.g., data message) from participant
        let data_message = Message::builder()
            .source(participant.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(session_id)
            .message_id(rand::random::<u32>())
            .build_publish()
            .unwrap();

        sender.notify_received_activity(&data_message);

        // Counter should be reset to 0
        if let Some(hs) = &sender.heartbeat_state {
            assert_eq!(hs.missed_heartbeats.get(&participant).copied(), Some(0));
        } else {
            panic!("Heartbeat state should be initialized");
        }
    }
}
