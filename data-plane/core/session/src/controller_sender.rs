// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use display_error_chain::ErrorChainExt;
use slim_datapath::{
    api::{CommandPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType},
    messages::Name,
};
use tokio::sync::mpsc::Sender;
use tracing::debug;

use crate::{
    SessionError, Transmitter,
    common::SessionMessage,
    timer::Timer,
    timer_factory::{TimerFactory, TimerSettings},
    transmitter::SessionTransmitter,
};

/// Ping interval.
pub const PING_INTERVAL: Duration = Duration::from_secs(10);
/// Maximum number of consecutive ping failures before a participant is considered disconnected.
const MAX_PING_FAILURE: u32 = 3;
/// Synthetic moderator name used by participants to track ping reception
static MODERATOR_NAME: std::sync::LazyLock<Name> =
    std::sync::LazyLock::new(|| Name::from_strings(["agntcy", "ns", "moderator"]));

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
    missing_pings: HashMap<Name, u32>,

    /// The ping timer
    /// this timer is used only for the pings and it is not connected to
    /// a specific message, it is used to send new pings periodically
    ping_timer: Timer,
}

pub struct ControllerSender {
    /// timer factory to crate timers for acks
    timer_factory: TimerFactory,

    /// local name to be removed in the missing replies set
    local_name: Name,

    /// group name is set on the first join request message
    /// in p2p session is equal to the remote name of the join request
    /// while in multicast session is specified in the payload
    group_name: Option<Name>,

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
    group_list: HashSet<Name>,

    /// send packets to slim or the app
    tx: SessionTransmitter,

    /// send message to the session controller
    tx_session: Sender<SessionMessage>,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ControllerSenderDrainStatus,
}

impl ControllerSender {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timer_settings: TimerSettings,
        local_name: Name,
        session_type: ProtoSessionType,
        session_id: u32,
        ping_interval: Option<Duration>,
        initiator: bool,
        tx: SessionTransmitter,
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
            tx,
            tx_session: tx_signals,
            draining_state: ControllerSenderDrainStatus::NotDraining,
        }
    }

    // helper function to update local state based on the message type received
    fn update_local_state(&mut self, message: &Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            slim_datapath::api::ProtoSessionMessageType::GroupWelcome => {
                // update the group list on welcome messages
                // adding the new participant to the list
                debug!(
                    participant = %message.get_dst(),
                    "adding participant to group on welcome message"
                );
                self.group_list.insert(message.get_dst());
            }
            slim_datapath::api::ProtoSessionMessageType::LeaveRequest => {
                // update the group list on leave requests
                // removing the participant from the list
                debug!(
                    participant = %message.get_dst(),
                    "removing participant from group on leave request"
                );
                self.group_list.remove(&message.get_dst());

                // remove also the missing_pings state if present
                if let Some(ps) = self.ping_state.as_mut() {
                    ps.missing_pings.remove(&message.get_dst());
                }
            }
            slim_datapath::api::ProtoSessionMessageType::JoinRequest => {
                // setup the group name if not set yet
                if self.group_name.is_none() {
                    if self.session_type == ProtoSessionType::PointToPoint {
                        // in p2p session the group name is equal to the remote name
                        // in the join request message
                        debug!(
                            destination = %message.get_dst(),
                            "update group name on join request message for p2p session",
                        );
                        self.group_name = Some(message.get_dst());
                    } else {
                        // in multicast session the group name is specified in the
                        // payload of the message
                        let name = message
                            .extract_join_request()?
                            .channel
                            .as_ref()
                            .ok_or(SessionError::MissingGroupNameInJoinRequest)?;
                        let group_name = Name::from(name);
                        debug!(
                            destination = %group_name,
                            "update group name on join request message for multicast session",
                        );
                        self.group_name = Some(group_name);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn on_message(&mut self, message: &Message) -> Result<(), SessionError> {
        if self.draining_state == ControllerSenderDrainStatus::Completed {
            return Err(SessionError::SessionDrainingDrop);
        }

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
            slim_datapath::api::ProtoSessionMessageType::Ping => self.on_ping_message(message),
            slim_datapath::api::ProtoSessionMessageType::GroupAdd => {
                // compute the list of participants that needs to send an ack
                // remove the local name as we are not waiting for any reply from the local name
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

                self.on_send_message(message, missing_replies).await?;
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

                // remove the endpoint also from the group list
                let payload = message.extract_group_remove()?;

                let to_remove = Name::from(
                    payload
                        .removed_participant
                        .as_ref()
                        .ok_or(SessionError::MissingRemovedParticipantInGroupRemove)?,
                );

                self.group_list.remove(&to_remove);

                self.on_send_message(message, missing_replies).await?;
            }
            slim_datapath::api::ProtoSessionMessageType::GroupClose => {
                // compute the list of participants that needs to send an ack
                let missing_replies = self
                    .group_list
                    .iter()
                    .filter(|name| *name != &self.local_name)
                    .cloned()
                    .collect::<HashSet<_>>();

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
        self.tx.send_to_slim::<(), ()>(Ok(message.clone()), None).await
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

    pub(crate) async fn on_timer_timeout(
        &mut self,
        id: u32,
        msg_type: ProtoSessionMessageType,
    ) -> Result<(), SessionError> {
        debug!(%id, ?msg_type, "timeout for message");

        // check if the timeout is related to a ping
        if self.ping_state.is_some() && msg_type == ProtoSessionMessageType::Ping {
            return self.handle_ping_timeout(id).await;
        }

        // the timer is not related to a ping, resent the message if possible
        if let Some(pending) = self.pending_replies.get(&id) {
            return self.tx.send_to_slim::<(), ()>(Ok(pending.message.clone()), None).await;
        };

        Err(SessionError::TimerNotFound(id))
    }

    async fn handle_ping_timeout(&mut self, id: u32) -> Result<(), SessionError> {
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
            return Ok(());
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

                // Send the ping message to slim
                self.tx.send_to_slim::<(), ()>(Ok(ping.clone()), None).await?;

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
                return self.tx.send_to_slim::<(), ()>(Ok(ping_message), None).await;
            }
        }

        Ok(())
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

    pub fn on_failure(&mut self, id: u32, msg_type: ProtoSessionMessageType) {
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
                return;
            }
        }

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

    pub fn remove_participant(&mut self, name: &Name) {
        // this is used only by the moderator when a participant closes its
        // session remotely. This remove is needed so that the moderator
        // will not expect acks from the participant that is leaving the
        // group during the group update phase
        if self.initiator {
            self.group_list.remove(name);
        }
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
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // Create a discovery request message
        let payload = CommandPayload::builder().discovery_request(None);

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
            .on_timer_timeout(1, ProtoSessionMessageType::DiscoveryRequest)
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
        let channel = Name::from_strings(["org", "ns", "channel"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // Create a join request message
        let payload = CommandPayload::builder().join_request(
            false,                 // enable_mls
            None,                  // max_retries
            None,                  // timer_duration
            Some(channel.clone()), // channel
        );

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
            .on_timer_timeout(1, ProtoSessionMessageType::JoinRequest)
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
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // Create a leave request message
        let payload = CommandPayload::builder().leave_request(None);

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
            .on_timer_timeout(1, ProtoSessionMessageType::LeaveRequest)
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
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // Create a group welcome message
        let participant = Name::from_strings(["org", "ns", "participant"]);
        let payload = CommandPayload::builder()
            .group_welcome(vec![participant.clone(), source.clone()], None);

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
            .on_timer_timeout(1, ProtoSessionMessageType::GroupWelcome)
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
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // First add participant2 to establish a group with 2 members (source + participant2)
        let participant2 = Name::from_strings(["org", "ns", "participant2"]);
        sender.group_list.insert(participant2.clone());

        // Now create a group add message to add participant1
        // This should wait for acks from both participant2 (already in group) and participant1 (being added)
        let participant1 = Name::from_strings(["org", "ns", "participant1"]);
        let payload = CommandPayload::builder().group_add(
            participant1.clone(),
            vec![participant1.clone(), participant2.clone(), source.clone()],
            None, // mls_commit
        );

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
            .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
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
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            None,
            false,
            tx,
            tx_signal,
        );

        // First add participant2 to establish a group with 2 members (source + participant2)
        let participant2 = Name::from_strings(["org", "ns", "participant2"]);
        sender.group_list.insert(participant2.clone());

        // Now create a group add message to add participant1
        // This should wait for acks from both participant2 (already in group) and participant1 (being added)
        let participant1 = Name::from_strings(["org", "ns", "participant1"]);
        let payload = CommandPayload::builder().group_add(
            participant1.clone(),
            vec![participant1.clone(), participant2.clone(), source.clone()],
            None, // mls
        );

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
            .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
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

        // notify the timeout to the sender (this will resend the message)
        sender
            .on_timer_timeout(1, ProtoSessionMessageType::GroupAdd)
            .await
            .expect("error re-sending the add");

        // Consume the retransmitted message from the channel
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmitted message")
            .expect("channel closed")
            .expect("error message");
        assert_eq!(received, update);

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

    #[tokio::test]
    #[traced_test]
    async fn test_ping_with_retransmissions_and_disconnection() {
        // Test ping functionality:
        // - Ping sent every 1 second
        // - Ping retry every 400ms
        // - First ping gets reply
        // - Subsequent pings get no reply (2 retransmissions per interval)
        // - After 3 failed ping intervals, participant is considered disconnected

        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(100);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let participant = Name::from_strings(["org", "ns", "participant"]);
        let session_id = 1;

        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true,
            tx,
            tx_signal,
        );

        // Add participant to the group and set group name
        sender.group_list.insert(participant.clone());
        sender.group_name = Some(participant.clone());

        // === PING INTERVAL 1: First ping gets a reply ===

        // Wait for the first ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for first ping interval")
            .expect("channel closed");

        let first_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending first ping");

        // Verify ping was sent to slim
        let first_ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for first ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            first_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        // Send a ping reply from the participant (ping with same message ID)
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

        // Verify no retransmission happens (participant replied)
        let res = timeout(Duration::from_millis(500), rx_slim.recv()).await;
        assert!(
            res.is_err(),
            "Expected no retransmission after successful ping reply"
        );

        // === PING INTERVAL 2: No reply, expect 2 retransmissions ===

        // Wait for the second ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for second ping interval")
            .expect("channel closed");

        let second_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending second ping");

        // Verify ping was sent
        let second_ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for second ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            second_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        // Wait for first retransmission timeout (400ms)
        let timeout_msg = timeout(Duration::from_millis(500), rx_signal.recv())
            .await
            .expect("timeout waiting for first retransmission")
            .expect("channel closed");

        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, second_ping.get_id());
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
            }
            _ => panic!("Expected TimerTimeout for ping retransmission"),
        };

        // Trigger retransmission
        sender
            .on_timer_timeout(second_ping.get_id(), ProtoSessionMessageType::Ping)
            .await
            .expect("error retransmitting ping");

        // Verify retransmission was sent
        let retrans1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmission")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(retrans1.get_id(), second_ping.get_id());

        // Wait for second retransmission timeout (400ms)
        let timeout_msg = timeout(Duration::from_millis(500), rx_signal.recv())
            .await
            .expect("timeout waiting for second retransmission")
            .expect("channel closed");

        match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_id, second_ping.get_id());
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
            }
            _ => panic!("Expected TimerTimeout for ping retransmission"),
        };

        // Trigger second retransmission
        sender
            .on_timer_timeout(second_ping.get_id(), ProtoSessionMessageType::Ping)
            .await
            .expect("error retransmitting ping");

        // Verify second retransmission was sent
        let retrans2 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for second retransmission")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(retrans2.get_id(), second_ping.get_id());

        // === PING INTERVAL 3: No reply, expect 2 retransmissions ===

        // Wait for the third ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for third ping interval")
            .expect("channel closed");

        let third_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(third_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending third ping");

        // Verify ping was sent
        let third_ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for third ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            third_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        // Wait for and process 2 retransmissions
        for i in 0..2 {
            let timeout_msg = timeout(Duration::from_millis(500), rx_signal.recv())
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for retransmission {}", i + 1))
                .expect("channel closed");

            match timeout_msg {
                SessionMessage::TimerTimeout {
                    message_id,
                    message_type,
                    ..
                } => {
                    assert_eq!(message_id, third_ping.get_id());
                    assert_eq!(message_type, ProtoSessionMessageType::Ping);
                }
                _ => panic!("Expected TimerTimeout for ping retransmission"),
            };

            sender
                .on_timer_timeout(third_ping.get_id(), ProtoSessionMessageType::Ping)
                .await
                .expect("error retransmitting ping");

            let _ = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for retransmission")
                .expect("channel closed")
                .expect("error message");
        }

        // === PING INTERVAL 4: No reply, expect 2 retransmissions, then disconnection ===

        // Wait for the fourth ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for fourth ping interval")
            .expect("channel closed");

        let fourth_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(fourth_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending fourth ping");

        // Verify ping was sent
        let fourth_ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for fourth ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            fourth_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        // Wait for and process 2 retransmissions
        for i in 0..2 {
            let timeout_msg = timeout(Duration::from_millis(500), rx_signal.recv())
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for retransmission {}", i + 1))
                .expect("channel closed");

            match timeout_msg {
                SessionMessage::TimerTimeout {
                    message_id,
                    message_type,
                    ..
                } => {
                    assert_eq!(message_id, fourth_ping.get_id());
                    assert_eq!(message_type, ProtoSessionMessageType::Ping);
                }
                _ => panic!("Expected TimerTimeout for ping retransmission"),
            };

            sender
                .on_timer_timeout(fourth_ping.get_id(), ProtoSessionMessageType::Ping)
                .await
                .expect("error retransmitting ping");

            let _ = timeout(Duration::from_millis(100), rx_slim.recv())
                .await
                .expect("timeout waiting for retransmission")
                .expect("channel closed")
                .expect("error message");
        }

        // === PING INTERVAL 5: Wait for next interval to reach disconnection threshold ===

        // Wait for the fifth ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for fifth ping interval")
            .expect("channel closed");

        let fifth_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // This interval timeout will:
        // 1. See that fourth ping got no reply
        // 2. Increment missing_pings counter for participant to 3
        // 3. Detect disconnection (counter >= MAX_PING_FAILURE)
        // 4. Send a new ping
        sender
            .on_timer_timeout(fifth_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling fifth ping interval");

        // Verify the participant was detected as disconnected and removed
        // Timeline:
        // - Interval 1: ping sent, reply received, counter cleared to 0
        // - Interval 2: previous ping got reply, counter stays 0, new ping sent
        // - Interval 3: previous ping got NO reply, counter becomes 1, new ping sent
        // - Interval 4: previous ping got NO reply, counter becomes 2, new ping sent
        // - Interval 5: previous ping got NO reply, counter becomes 3, disconnection detected
        if let Some(ping_state) = &sender.ping_state {
            // Participant should be removed from missing_pings after disconnection
            assert_eq!(
                ping_state.missing_pings.get(&participant),
                None,
                "Participant should be removed from missing_pings after disconnection"
            );
        } else {
            panic!("Ping state should be initialized");
        }

        // Verify participant was removed from group_list
        assert!(
            !sender.group_list.contains(&participant),
            "Participant should be removed from group_list after disconnection"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_participant_detects_moderator_disconnection() {
        // Test participant-side moderator disconnection detection:
        // - Participant receives pings from moderator initially
        // - Moderator stops sending pings (simulating crash/network failure)
        // - After 3 ping intervals without receiving pings, participant detects disconnection
        // - ParticipantDisconnected message is sent with name: None

        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);

        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(100);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let participant_name = Name::from_strings(["org", "ns", "participant"]);
        let moderator_name = Name::from_strings(["org", "ns", "moderator"]);
        let channel_name = Name::from_strings(["org", "ns", "channel"]);
        let session_id = 1;

        // Create participant (initiator=false)
        let mut sender = ControllerSender::new(
            settings,
            participant_name.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            false, // participant, not initiator
            tx,
            tx_signal,
        );

        // === PING INTERVAL 1: Moderator sends ping, participant receives it ===

        // Wait for first ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for first ping interval")
            .expect("channel closed");

        let first_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Simulate receiving a ping from moderator
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

        // Process the timeout - should reset state since ping was received
        sender
            .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling first ping timeout");

        // Verify state was reset (no disconnection detected)
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.len(),
                0,
                "Missing pings should be cleared after receiving ping"
            );
        }

        // === PING INTERVAL 2: No ping received, counter increments to 1 ===

        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for second ping interval")
            .expect("channel closed");

        let second_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Don't send a ping this time (moderator is down)
        sender
            .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling second ping timeout");

        // Verify counter incremented to 1
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.get(&MODERATOR_NAME).copied(),
                Some(1),
                "Missing ping counter should be 1"
            );
        }

        // === PING INTERVAL 3: No ping received, counter increments to 2 ===

        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for third ping interval")
            .expect("channel closed");

        let third_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        sender
            .on_timer_timeout(third_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling third ping timeout");

        // Verify counter incremented to 2
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.get(&MODERATOR_NAME).copied(),
                Some(2),
                "Missing ping counter should be 2"
            );
        }

        // === PING INTERVAL 4: No ping received, counter reaches 3, disconnection detected ===

        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for fourth ping interval")
            .expect("channel closed");

        let fourth_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        sender
            .on_timer_timeout(fourth_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling fourth ping timeout");

        // Verify ParticipantDisconnected message was sent with name: None (indicating moderator)
        // Note: The message is sent via try_send, so it should already be in the channel
        let disconnect_msg = rx_signal.try_recv();

        match disconnect_msg {
            Ok(SessionMessage::ParticipantDisconnected { name }) => {
                assert_eq!(
                    name, None,
                    "ParticipantDisconnected should have name: None for moderator disconnection"
                );
            }
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
        // Test that removing a participant before ping reply clears their missing ping state:
        // - Start with moderator + 2 participants
        // - Send a ping to both participants
        // - Remove one participant before ping reply is received
        // - Verify removed participant is not tracked in missing_pings
        // - Receive reply from remaining participant
        // - Verify ping state is properly cleared

        // Use very high duration to avoid retransmission timeouts during test
        let settings = TimerSettings::constant(Duration::from_secs(1000)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(100);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let moderator = Name::from_strings(["org", "ns", "moderator"]);
        let participant1 = Name::from_strings(["org", "ns", "participant1"]);
        let participant2 = Name::from_strings(["org", "ns", "participant2"]);
        let group_name = Name::from_strings(["org", "ns", "group"]);
        let session_id = 1;

        // Create moderator (initiator=true) with 2 participants
        let mut sender = ControllerSender::new(
            settings,
            moderator.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true, // initiator (moderator)
            tx,
            tx_signal,
        );

        // Set up group with 2 participants
        sender.group_name = Some(group_name.clone());
        sender.group_list.insert(moderator.clone());
        sender.group_list.insert(participant1.clone());
        sender.group_list.insert(participant2.clone());

        // === PING INTERVAL 1: Send ping to both participants ===

        // Wait for first ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for first ping interval")
            .expect("channel closed");

        let first_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger ping send
        sender
            .on_timer_timeout(first_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending first ping");

        // Verify ping was sent
        let ping_msg = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            ping_msg.get_session_message_type(),
            ProtoSessionMessageType::Ping
        );

        let ping_message_id = ping_msg.get_id();

        // Verify both participants are in missing_replies
        if let Some(ping_state) = &sender.ping_state {
            if let Some(ping) = &ping_state.ping {
                assert_eq!(
                    ping.missing_replies.len(),
                    2,
                    "Should be waiting for replies from both participants"
                );
                assert!(
                    ping.missing_replies.contains(&participant1),
                    "Participant1 should be in missing_replies"
                );
                assert!(
                    ping.missing_replies.contains(&participant2),
                    "Participant2 should be in missing_replies"
                );
            } else {
                panic!("Ping should be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }

        // === Remove participant1 before ping reply is received ===
        sender.remove_participant(&participant1);

        // Verify participant1 was removed from group_list
        assert!(
            !sender.group_list.contains(&participant1),
            "Participant1 should be removed from group_list"
        );
        assert!(
            sender.group_list.contains(&participant2),
            "Participant2 should still be in group_list"
        );

        // === Receive ping reply from participant2 (the remaining participant) ===
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

        // Verify participant2 was removed from missing_replies after receiving the ping
        // but participant1 is still there
        if let Some(ping_state) = &sender.ping_state {
            if let Some(ping) = &ping_state.ping {
                assert_eq!(
                    ping.missing_replies.len(),
                    1,
                    "Should still be waiting for reply from participant1"
                );
                assert!(
                    ping.missing_replies.contains(&participant1),
                    "Participant1 should still be in missing_replies since they haven't replied"
                );
                assert!(
                    !ping.missing_replies.contains(&participant2),
                    "Participant2 should be removed from missing_replies after replying"
                );
            } else {
                panic!("Ping should still be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }

        // === Wait for next ping interval to trigger handle_ping_state ===
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for second ping interval")
            .expect("channel closed");

        let second_ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the second ping interval
        sender
            .on_timer_timeout(second_ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error handling second ping interval");

        // Verify participant1 is NOT in missing_pings (because they were removed from group_list)
        // Only participants still in group_list should be tracked in missing_pings
        if let Some(ping_state) = &sender.ping_state {
            assert_eq!(
                ping_state.missing_pings.get(&participant1),
                None,
                "Removed participant1 should NOT be in missing_pings"
            );
            assert_eq!(
                ping_state.missing_pings.len(),
                0,
                "No participants should be in missing_pings since participant2 replied and participant1 was removed"
            );
        } else {
            panic!("Ping state should be initialized");
        }

        // Verify a new ping was sent to the remaining participant
        let second_ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for second ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            second_ping.get_session_message_type(),
            ProtoSessionMessageType::Ping,
            "Second ping should be sent"
        );

        // Verify the new ping only expects reply from participant2
        if let Some(ping_state) = &sender.ping_state {
            if let Some(ping) = &ping_state.ping {
                assert_eq!(
                    ping.missing_replies.len(),
                    1,
                    "Should only be waiting for reply from participant2"
                );
                assert!(
                    ping.missing_replies.contains(&participant2),
                    "Participant2 should be in missing_replies"
                );
                assert!(
                    !ping.missing_replies.contains(&participant1),
                    "Participant1 should NOT be in missing_replies"
                );
            } else {
                panic!("Ping should be set");
            }
        } else {
            panic!("Ping state should be initialized");
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ping_destination_p2p_session() {
        // Test that ping messages in P2P sessions use the remote participant name as destination
        // The group_name should be set to the remote participant's name from the JoinRequest

        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(100);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let remote = Name::from_strings(["org", "ns", "remote"]);
        let session_id = 1;

        // Create P2P session with ping enabled
        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::PointToPoint,
            session_id,
            Some(ping_interval),
            true, // initiator
            tx,
            tx_signal,
        );

        // Simulate sending a join request to establish group_name
        let payload = CommandPayload::builder().join_request(
            false, // enable_mls
            None,  // max_retries
            None,  // timer_duration
            None,  // channel (None for P2P)
        );

        let join_request = Message::builder()
            .source(source.clone())
            .destination(remote.clone())
            .identity("")
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(session_id)
            .message_id(1)
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        sender
            .on_message(&join_request)
            .await
            .expect("error sending join request");

        // Consume the join request message
        let join_msg = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for join request")
            .expect("channel closed")
            .expect("error message");

        // Verify group_name was set to the remote participant's name
        assert_eq!(
            sender.group_name,
            Some(remote.clone()),
            "Group name should be set to remote participant name in P2P session"
        );

        // Send join reply to clear pending state
        let reply_payload = CommandPayload::builder().join_reply(None);
        let join_reply = Message::builder()
            .source(remote.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(join_msg.get_id())
            .payload(reply_payload.as_content())
            .build_publish()
            .unwrap();

        sender
            .on_message(&join_reply)
            .await
            .expect("error sending join reply");

        // Add remote to group list (normally done on welcome)
        sender.group_list.insert(remote.clone());

        // Wait for the first ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for first ping interval")
            .expect("channel closed");

        let ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending ping");

        // Verify ping was sent with correct destination
        let ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            ping.get_session_message_type(),
            ProtoSessionMessageType::Ping,
            "Message should be a ping"
        );
        assert_eq!(
            ping.get_dst(),
            remote,
            "Ping destination should be the remote participant name in P2P session"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_ping_destination_multicast_session() {
        // Test that ping messages in multicast sessions use the channel/group name as destination
        // The group_name should be set to the channel name from the JoinRequest payload

        let settings = TimerSettings::constant(Duration::from_millis(400)).with_max_retries(3);
        let ping_interval = Duration::from_millis(1000);

        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(100);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(100);

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["org", "ns", "source"]);
        let channel_name = Name::from_strings(["org", "ns", "channel"]);
        let participant = Name::from_strings(["org", "ns", "participant"]);
        let session_id = 1;

        // Create multicast session with ping enabled
        let mut sender = ControllerSender::new(
            settings,
            source.clone(),
            ProtoSessionType::Multicast,
            session_id,
            Some(ping_interval),
            true, // initiator
            tx,
            tx_signal,
        );

        // Simulate sending a join request with channel name in payload
        let payload = CommandPayload::builder().join_request(
            false,                      // enable_mls
            None,                       // max_retries
            None,                       // timer_duration
            Some(channel_name.clone()), // channel name
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
            .payload(payload.as_content())
            .build_publish()
            .unwrap();

        sender
            .on_message(&join_request)
            .await
            .expect("error sending join request");

        // Consume the join request message
        let join_msg = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for join request")
            .expect("channel closed")
            .expect("error message");

        // Verify group_name was set to the channel name from payload
        assert_eq!(
            sender.group_name,
            Some(channel_name.clone()),
            "Group name should be set to channel name from JoinRequest payload in multicast session"
        );

        // Send join reply to clear pending state
        let reply_payload = CommandPayload::builder().join_reply(None);
        let join_reply = Message::builder()
            .source(participant.clone())
            .destination(source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinReply)
            .session_id(session_id)
            .message_id(join_msg.get_id())
            .fanout(256)
            .payload(reply_payload.as_content())
            .build_publish()
            .unwrap();

        sender
            .on_message(&join_reply)
            .await
            .expect("error sending join reply");

        // Add participant to group list (normally done on welcome)
        sender.group_list.insert(participant.clone());

        // Wait for the first ping interval timeout
        let timeout_msg = timeout(Duration::from_millis(1100), rx_signal.recv())
            .await
            .expect("timeout waiting for first ping interval")
            .expect("channel closed");

        let ping_id = match timeout_msg {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(message_type, ProtoSessionMessageType::Ping);
                message_id
            }
            _ => panic!("Expected TimerTimeout for ping interval"),
        };

        // Trigger the ping send
        sender
            .on_timer_timeout(ping_id, ProtoSessionMessageType::Ping)
            .await
            .expect("error sending ping");

        // Verify ping was sent with correct destination
        let ping = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ping")
            .expect("channel closed")
            .expect("error message");

        assert_eq!(
            ping.get_session_message_type(),
            ProtoSessionMessageType::Ping,
            "Message should be a ping"
        );
        assert_eq!(
            ping.get_dst(),
            channel_name,
            "Ping destination should be the channel/group name in multicast session"
        );
    }
}
