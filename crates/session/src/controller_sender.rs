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

/// Ping interval.
pub const PING_INTERVAL: Duration = Duration::from_secs(10);
/// Maximum number of consecutive ping failures before a participant is considered disconnected.
const MAX_PING_FAILURE: u32 = 3;
/// Synthetic moderator name used by participants to track ping reception
static MODERATOR_NAME: std::sync::LazyLock<ProtoName> =
    std::sync::LazyLock::new(|| ProtoName::from_strings(["agntcy", "ns", "moderator"]));

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

struct PingState {
    /// Current pending ping
    /// Maybe empty if none is connected to the channel
    /// Used only if this endpoint is sending ping messages
    ping: Option<PendingReply>,

    /// This indicates if at least one ping message was received
    /// during the last ping timer. It is used only by participants
    /// and set to true on the ping reception
    received_ping: bool,

    /// Ping timer factor set to create ping related timers
    #[allow(dead_code)]
    ping_timer_factory: TimerFactory,

    /// List of potential disconnected endpoint
    /// Initiation/Moderator mode:
    /// If an endpoint does not reply to latest N pings it is considered
    /// disconnected and the session controller is notified.
    /// The map keeps track of the name and the number of missing ping replies
    /// Participant mode:
    /// The map keeps track only of the moderator and checks for how many
    /// interval the moderator was silent
    missing_pings: HashMap<ProtoName, u32>,

    /// The ping timer
    /// this timer is used only for the pings and it is not connected to
    /// a specific message, it is used to send new pings periodically
    ping_timer: Timer,
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

    /// ping state
    /// by default is None, start only if a duration is set
    ping_state: Option<PingState>,

    /// set to true if the participant is an initiator
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
        ping_interval: Option<Duration>,
        initiator: bool,
        tx_signals: Sender<SessionMessage>,
    ) -> Self {
        let mut list = HashSet::new();
        list.insert(local_name.clone());

        let ping_state = if let Some(interval) = ping_interval {
            // we need to setup the timer for the ping
            let settings =
                TimerSettings::new(interval, None, None, crate::timer::TimerType::Constant);
            let ping_timer_factory = TimerFactory::new(settings, tx_signals.clone());
            let ping_timer = ping_timer_factory.create_and_start_timer(
                rand::random::<u32>(),
                slim_datapath::api::ProtoSessionMessageType::Ping,
                None,
            );
            Some(PingState {
                ping: None,
                received_ping: false,
                ping_timer_factory,
                missing_pings: HashMap::new(),
                ping_timer,
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
            ping_state,
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
    }

    pub fn remove_participant(&mut self, name: &ProtoName) {
        debug!(
            participant = %name,
            "removing participant from group on controller sender"
        );
        self.group_list.remove(name);

        // remove also the missing_pings state if present
        if let Some(ps) = self.ping_state.as_mut() {
            ps.missing_pings.remove(name);
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
            | slim_datapath::api::ProtoSessionMessageType::GroupWelcome
            | slim_datapath::api::ProtoSessionMessageType::RejoinRequest => {
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
            | slim_datapath::api::ProtoSessionMessageType::RejoinReply
            | slim_datapath::api::ProtoSessionMessageType::GroupAck => {
                self.on_reply_message(message);
            }
            slim_datapath::api::ProtoSessionMessageType::GroupNack => {
                // in case on Nack we stop the timer as for the Acks
                // and we leave the application/controller decide what
                // to do to handle it
                self.on_reply_message(message);
            }
            slim_datapath::api::ProtoSessionMessageType::Ping => self.on_ping_message(message),
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
            slim_datapath::api::ProtoSessionMessageType::GroupProposal => todo!(),
            slim_datapath::api::ProtoSessionMessageType::UpdateParticipantState => {
                // compute the list of participants that needs to send an ack
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();
                debug!(
                    "send update state with message id {}, expected acks from {:?}",
                    message.get_id(),
                    missing_replies
                );
                output.extend(self.on_send_message(message, missing_replies)?);
            }
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

    fn on_ping_message(&mut self, message: &Message) {
        debug!(id = %message.get_id(), "received ping message");
        if self.initiator {
            // if this is an initiator update the missing acks
            if let Some(ping_state) = &mut self.ping_state
                && let Some(ping) = &mut ping_state.ping
                && ping.timer.get_id() == message.get_id()
            {
                ping.missing_replies.remove(&message.get_source());
                if ping.missing_replies.is_empty() {
                    debug!("stop ping retransmissions for id {}", message.get_id());
                    ping.timer.stop()
                }
                return;
            }
        } else {
            // if this is a participant mark the reception of the message
            if let Some(ping_state) = &mut self.ping_state {
                ping_state.received_ping = true;
                return;
            }
        }

        debug!(id = %message.get_id(), "received a ping but the state is not set, ignore the message");
    }

    pub fn is_still_pending(&self, message_id: u32) -> bool {
        self.pending_replies.contains_key(&message_id)
    }

    pub(crate) fn on_timer_timeout(
        &mut self,
        id: u32,
        msg_type: ProtoSessionMessageType,
    ) -> Result<SessionOutput, SessionError> {
        debug!(%id, ?msg_type, "timeout for message");

        // check if the timeout is related to a ping
        if self.ping_state.is_some() && msg_type == ProtoSessionMessageType::Ping {
            return self.handle_ping_timeout(id);
        }

        // the timer is not related to a ping, resent the message if possible
        if let Some(pending) = self.pending_replies.get(&id) {
            let mut output = SessionOutput::new();
            output.push_slim(pending.message.clone());
            return Ok(output);
        };

        Err(SessionError::TimerNotFound(id))
    }

    fn handle_ping_timeout(&mut self, id: u32) -> Result<SessionOutput, SessionError> {
        // If this is a participant check if a message was received
        // during the last ping time
        if !self.initiator
            && let Some(ping_state) = &mut self.ping_state
        {
            if ping_state.received_ping {
                // reset the state
                debug!(%id, "received at least on ping message, reset the state");
                ping_state.received_ping = false;
                ping_state.missing_pings.clear();
            } else {
                // update the missing ping map and detect moderator disconnection
                debug!(%id, "missing ping message from moderator");
                let val = ping_state
                    .missing_pings
                    .entry(MODERATOR_NAME.clone())
                    .or_insert(0);
                *val += 1;
                if *val >= MAX_PING_FAILURE {
                    debug!("moderator got disconnected");
                    if let Err(e) = self
                        .tx_session
                        .try_send(SessionMessage::ParticipantDisconnected { name: None })
                    {
                        debug!(error = %e.chain(), "failed to send participant disconnected message");
                    }
                }
            }
            return Ok(SessionOutput::new());
        }

        // This is a initiator
        // Check if we need to handle ping timeout
        let should_handle_ping_interval = self
            .ping_state
            .as_ref()
            .map(|ps| ps.ping_timer.get_id() == id)
            .ok_or(SessionError::PingStateNotInitialized)?;

        if should_handle_ping_interval {
            debug!("ping interval timeout, check current group state");
            // the timeout is related to the ping interval timer
            // check if we sent a ping before and if there are still pending acks to the ping
            self.handle_ping_state();

            // completely reset the ping if needed
            self.ping_state.as_mut().map(|s| s.ping.take());

            if self.group_list.len() > 1
                && let Some(group_name) = &self.group_name
            {
                // someone is connected to the channel, send the ping
                // create the message
                let ping_id = rand::random::<u32>();
                let mut builder = Message::builder()
                    .source(self.local_name.clone())
                    .destination(group_name.clone())
                    .identity("")
                    .session_type(self.session_type)
                    .session_message_type(ProtoSessionMessageType::Ping)
                    .session_id(self.session_id)
                    .message_id(ping_id)
                    .payload(CommandPayload::builder().ping().as_content());

                if self.session_type == ProtoSessionType::Multicast {
                    builder = builder.fanout(256);
                }

                let ping = builder.build_publish()?;

                debug!(id = %ping_id, "send a new ping");

                // set the ping missing replies state
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                let mut output = SessionOutput::new();
                output.push_slim(ping.clone());

                if let Some(ping_state) = self.ping_state.as_mut() {
                    ping_state.ping = Some(PendingReply {
                        missing_replies,
                        message: ping,
                        // the ping message should be resent like all the other command message
                        // if some remote participant do no replies. The ping_state.ping_timer_factory
                        // is used only to recreate the message periodically
                        timer: self.timer_factory.create_and_start_timer(
                            ping_id,
                            ProtoSessionMessageType::Ping,
                            None,
                        ),
                    });
                }

                return Ok(output);
            }
        } else {
            // most likely the timeout is related to the ping message itself so
            // we need to send it again
            let message_to_send = self
                .ping_state
                .as_ref()
                .and_then(|ps| ps.ping.as_ref())
                .map(|p| p.message.clone());

            if let Some(ping_message) = message_to_send {
                debug!(%id, "ping message timeout, send it again");
                // simply resend the message
                let mut output = SessionOutput::new();
                output.push_slim(ping_message);
                return Ok(output);
            }
        }

        Ok(SessionOutput::new())
    }

    /// Handle ping state by updating missing_pings and checking for disconnections
    fn handle_ping_state(&mut self) {
        let ping_state = self
            .ping_state
            .as_mut()
            .expect("ping_state should be initialized");
        let Some(mut ping) = ping_state.ping.take() else {
            return;
        };

        // stop the timer for ping retransmission
        ping.timer.stop();

        // if all participants replied to the ping, reset the
        // missing_pings map otherwise try to see if someone got disconnected
        if ping.missing_replies.is_empty() {
            debug!("all ping received, nobody got disconnected");
            ping_state.missing_pings.clear();
        } else {
            // update missing_pings
            for p in &ping.missing_replies {
                debug!(from = %p, "missing ping reply from");
                // add the non reply participant to the missing pings map
                // only if it is still connected to the group
                if self.group_list.contains(p) {
                    *ping_state.missing_pings.entry(p.clone()).or_insert(0) += 1;
                }
            }

            // check for disconnected participants and notify, then remove them
            ping_state.missing_pings.retain(|k, v| {
                if *v >= MAX_PING_FAILURE {
                    debug!(participant = %k, "participant got disconnected");
                    self.group_list.remove(k);
                    if let Err(e) =
                        self.tx_session
                            .try_send(SessionMessage::ParticipantDisconnected {
                                name: Some(k.clone()),
                            })
                    {
                        debug!(error = %e.chain(), "failed to send participant disconnected message");
                    }
                    false // remove from missing_pings
                } else {
                    true // keep in missing_pings
                }
            });
        }
    }

    pub fn on_failure(&mut self, id: u32, msg_type: ProtoSessionMessageType) -> Vec<ProtoName> {
        if msg_type == ProtoSessionMessageType::Ping {
            // the only timer that can fail is the one related to the ping retransmissions
            let should_handle = if let Some(ping_state) = &self.ping_state {
                ping_state
                    .ping
                    .as_ref()
                    .map(|ping| ping.timer.get_id() == id)
                    .unwrap_or(false)
            } else {
                false
            };

            if should_handle {
                // reset the pending ping state and wait for the next one to be sent
                debug!(%id, "ping message timer failure, update ping state");
                self.handle_ping_state();
            } else {
                debug!("got message failure for unknown ping, ignore it");
                return Vec::new();
            }
        }

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
                .on_timer_timeout(1, ProtoSessionMessageType::DiscoveryRequest)
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
                .on_timer_timeout(1, ProtoSessionMessageType::JoinRequest)
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
                .on_timer_timeout(1, ProtoSessionMessageType::LeaveRequest)
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
                .on_timer_timeout(1, ProtoSessionMessageType::GroupWelcome)
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
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
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
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
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
                .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
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
    async fn test_ping_with_retransmissions_and_disconnection() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true,
            tx_signal,
        );

        sender.group_list.insert(participant.clone());
        sender.group_name = Some(participant.clone());

        let (first_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let first_ping = single_slim_message(
            sender
                .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending first ping"),
        );
        assert_eq!(
            first_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        let ping_reply = Message::builder()
            .source(participant.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Ping)
            .session_id(session_id)
            .message_id(first_ping.get_id())
            .payload(CommandPayload::builder().ping().as_content())
            .build_publish()
            .unwrap();
        sender.on_ping_message(&ping_reply);
        assert!(
            timeout(Duration::from_millis(500), rx_signal.recv())
                .await
                .is_err()
        );

        let (second_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let second_ping = single_slim_message(
            sender
                .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending second ping"),
        );
        assert_eq!(
            second_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        for _ in 0..2 {
            let (message_id, message_type) =
                expect_timeout(&mut rx_signal, Duration::from_millis(500)).await;
            assert_eq!(message_id, second_ping.get_id());
            assert_eq!(message_type, ProtoSessionMessageType::Ping);
            let retransmitted = single_slim_message(
                sender
                    .on_timer_timeout(second_ping.get_id(), ProtoSessionMessageType::Ping)
                    .expect("error retransmitting ping"),
            );
            assert_eq!(retransmitted.get_id(), second_ping.get_id());
        }

        let (third_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let third_ping = single_slim_message(
            sender
                .on_timer_timeout(third_ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending third ping"),
        );
        assert_eq!(
            third_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        for _ in 0..2 {
            let (message_id, message_type) =
                expect_timeout(&mut rx_signal, Duration::from_millis(500)).await;
            assert_eq!(message_id, third_ping.get_id());
            assert_eq!(message_type, ProtoSessionMessageType::Ping);
            let retransmitted = single_slim_message(
                sender
                    .on_timer_timeout(third_ping.get_id(), ProtoSessionMessageType::Ping)
                    .expect("error retransmitting ping"),
            );
            assert_eq!(retransmitted.get_id(), third_ping.get_id());
        }

        let (fourth_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let fourth_ping = single_slim_message(
            sender
                .on_timer_timeout(fourth_ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending fourth ping"),
        );
        assert_eq!(
            fourth_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        for _ in 0..2 {
            let (message_id, message_type) =
                expect_timeout(&mut rx_signal, Duration::from_millis(500)).await;
            assert_eq!(message_id, fourth_ping.get_id());
            assert_eq!(message_type, ProtoSessionMessageType::Ping);
            let retransmitted = single_slim_message(
                sender
                    .on_timer_timeout(fourth_ping.get_id(), ProtoSessionMessageType::Ping)
                    .expect("error retransmitting ping"),
            );
            assert_eq!(retransmitted.get_id(), fourth_ping.get_id());
        }

        let (fifth_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        assert_no_messages(
            sender
                .on_timer_timeout(fifth_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling fifth ping interval"),
        );

        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(ping_state.missing_pings.get(&participant), None);
        } else {
            panic!("Ping state should be initialized");
        }
        assert!(!sender.group_list.contains(&participant));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_participant_detects_moderator_disconnection() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let participant_name = ProtoName::from_strings(["org", "ns", "participant"]);
        let moderator_name = ProtoName::from_strings(["org", "ns", "moderator"]);
        let channel_name = ProtoName::from_strings(["org", "ns", "channel"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            participant_name.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            false,
            tx_signal,
        );

        let (first_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);

        let ping_from_moderator = Message::builder()
            .source(moderator_name.clone())
            .destination(channel_name.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Ping)
            .session_id(session_id)
            .message_id(rand::random::<u32>())
            .payload(CommandPayload::builder().ping().as_content())
            .build_publish()
            .unwrap();
        sender.on_ping_message(&ping_from_moderator);
        assert_no_messages(
            sender
                .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling first ping timeout"),
        );

        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(ping_state.missing_pings.len(), 0);
        }

        let (second_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        assert_no_messages(
            sender
                .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling second ping timeout"),
        );
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.get(&MODERATOR_NAME).copied(),
                Some(1)
            );
        }

        let (third_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        assert_no_messages(
            sender
                .on_timer_timeout(third_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling third ping timeout"),
        );
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.get(&MODERATOR_NAME).copied(),
                Some(2)
            );
        }

        let (fourth_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        assert_no_messages(
            sender
                .on_timer_timeout(fourth_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling fourth ping timeout"),
        );

        match rx_signal.try_recv() {
            Ok(SessionMessage::ParticipantDisconnected { name }) => assert_eq!(name, None),
            Ok(other) => panic!("Expected ParticipantDisconnected message, got {:?}", other),
            Err(e) => panic!(
                "Expected ParticipantDisconnected message, channel error: {:?}",
                e
            ),
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ping_with_two_participants_one_removed_before_reply() {
        let settings = TimerSettings::constant(Duration::from_secs(1000)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let moderator = ProtoName::from_strings(["org", "ns", "moderator"]);
        let participant1 = ProtoName::from_strings(["org", "ns", "participant1"]);
        let participant2 = ProtoName::from_strings(["org", "ns", "participant2"]);
        let group_name = ProtoName::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            moderator.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true,
            tx_signal,
        );

        sender.group_name = Some(group_name.clone());
        sender.group_list.insert(moderator.clone());
        sender.group_list.insert(participant1.clone());
        sender.group_list.insert(participant2.clone());

        let (first_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let ping_msg = single_slim_message(
            sender
                .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending first ping"),
        );
        assert_eq!(
            ping_msg.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );
        let ping_message_id = ping_msg.get_id();

        if let Some(ping_state) = &sender.ping_state {
            if let Some(ping) = &ping_state.ping {
                assert_eq!(ping.missing_replies.len(), 2);
                assert!(ping.missing_replies.contains(&participant1));
                assert!(ping.missing_replies.contains(&participant2));
            } else {
                panic!("Ping should be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }

        sender.remove_participant(&participant1);
        assert!(!sender.group_list.contains(&participant1));
        assert!(sender.group_list.contains(&participant2));

        let ping_reply = Message::builder()
            .source(participant2.clone())
            .destination(moderator.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Ping)
            .session_id(session_id)
            .message_id(ping_message_id)
            .payload(CommandPayload::builder().ping().as_content())
            .build_publish()
            .unwrap();
        sender.on_ping_message(&ping_reply);

        if let Some(ping_state) = &sender.ping_state {
            if let Some(ping) = &ping_state.ping {
                assert_eq!(ping.missing_replies.len(), 1);
                assert!(ping.missing_replies.contains(&participant1));
                assert!(!ping.missing_replies.contains(&participant2));
            } else {
                panic!("Ping should still be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }

        let (second_ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let second_ping = single_slim_message(
            sender
                .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
                .expect("error handling second ping interval"),
        );

        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(ping_state.missing_pings.get(&participant1), None);
            assert_eq!(ping_state.missing_pings.len(), 0);
            if let Some(ping) = &ping_state.ping {
                assert_eq!(ping.missing_replies.len(), 1);
                assert!(ping.missing_replies.contains(&participant2));
                assert!(!ping.missing_replies.contains(&participant1));
            } else {
                panic!("Ping should be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }

        assert_eq!(
            second_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ping_destination_p2p_session() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let remote = ProtoName::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::PointToPoint,
            session_id,
            Some(ping_interval),
            true,
            tx_signal,
        );

        let join_request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(
                CommandPayload::builder()
                    .join_request(None, None, None, None, None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();
        let join_msg = single_slim_message(
            sender
                .on_message(&join_request)
                .expect("error sending join request"),
        );
        assert_eq!(sender.group_name, Some(remote.clone()));

        let join_reply = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(join_msg.get_id())
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
        assert_no_messages(
            sender
                .on_message(&join_reply)
                .expect("error sending join reply"),
        );

        sender.group_list.insert(remote.clone());

        let (ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let ping = single_slim_message(
            sender
                .on_timer_timeout(ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending ping"),
        );
        assert_eq!(
            ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );
        assert_eq!(ping.get_dst(), remote);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ping_destination_multicast_session() {
        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let source = ProtoName::from_strings(["org", "ns", "source"]);
        let data_channel_name = ProtoName::from_strings(["org", "ns", "channel"]).with_id(1);
        let control_channel_name = ProtoName::from_strings(["org", "ns", "channel"]).with_id(2);
        let participant = ProtoName::from_strings(["org", "ns", "participant"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true,
            tx_signal,
        );

        let join_request = Message::builder()
            .source(source.clone())
            .destination(participant.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(session_id)
            .message_id(1)
            .fanout(256)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        None,
                        None,
                        Some(data_channel_name.clone()),
                        Some(control_channel_name.clone()),
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();
        let join_msg = single_slim_message(
            sender
                .on_message(&join_request)
                .expect("error sending join request"),
        );
        assert_eq!(sender.group_name, Some(control_channel_name.clone()));

        let join_reply = Message::builder()
            .source(participant.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(join_msg.get_id())
            .fanout(256)
            .payload(
                CommandPayload::builder()
                    .join_reply(
                        None,
                        Participant::new(participant.clone(), ParticipantSettings::bidirectional()),
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();
        assert_no_messages(
            sender
                .on_message(&join_reply)
                .expect("error sending join reply"),
        );

        sender.group_list.insert(participant.clone());

        let (ping_id, message_type) =
            expect_timeout(&mut rx_signal, Duration::from_millis(1100)).await;
        assert_eq!(message_type, ProtoSessionMessageType::Ping);
        let ping = single_slim_message(
            sender
                .on_timer_timeout(ping_id, ProtoSessionMessageType::Ping)
                .expect("error sending ping"),
        );
        assert_eq!(
            ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );
        assert_eq!(ping.get_dst(), control_channel_name);
    }
}
