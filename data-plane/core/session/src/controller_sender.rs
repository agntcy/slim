// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use slim_datapath::api::ProtoMessage as Message;
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

#[allow(dead_code)]
struct PendingReply {
    /// Number of missing replies
    missing_replies: u32,

    /// Message to resend in case of timeout
    message: Message,

    /// the timer
    timer: Timer,
}

#[allow(dead_code)]
pub struct ControllerSender {
    /// timer factory to crate timers for acks
    timer_factory: TimerFactory,

    /// list of pending replies for each control message
    pending_replies: HashMap<u32, PendingReply>,

    /// send packets to slim or the app
    tx: SessionTransmitter,

    /// drain state - when true, no new messages from app are accepted
    draining_state: ControllerSenderDrainStatus,
}

#[allow(dead_code)]
impl ControllerSender {
    pub fn new(
        timer_settings: TimerSettings,
        tx: SessionTransmitter,
        tx_signals: Sender<SessionMessage>,
    ) -> Self {
        ControllerSender {
            timer_factory: TimerFactory::new(timer_settings, tx_signals),
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
                self.on_send_message(message, 1).await?;
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
            slim_datapath::api::ProtoSessionMessageType::GroupUpdate => {
                // the number of acks expected for this message is equal
                // to the number of participants in the payload of the
                // packet. Notice that the controller process one update
                // after the other so it cannot happen that we send an
                // update and we remove a participant from the group at
                // the same time
                let payload = message
                    .get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_group_update_payload();
                let acks = payload.participant.len();
                // TODO: the actual number of acks to receive here may be paticipants - 1!!!
                self.on_send_message(message, acks as u32).await?;
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
        expected_replies: u32,
    ) -> Result<(), SessionError> {
        let id = message.get_id();
        let pending = PendingReply {
            missing_replies: expected_replies,
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
        let mut delete = false;
        if let Some(pending) = self.pending_replies.get_mut(&id) {
            debug!("try to remove {} from pending acks", id);
            pending.missing_replies -= 1;
            if pending.missing_replies == 0 {
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

    pub fn start_drain(&mut self) -> ControllerSenderDrainStatus {
        if self.pending_replies.is_empty() {
            debug!("closing controller sender");
            self.draining_state = ControllerSenderDrainStatus::Completed;
        } else {
            debug!("controller sender drain initiated");
            self.draining_state = ControllerSenderDrainStatus::Initiated;
        }
        self.draining_state.clone()
    }

    pub fn check_drain_completion(&self) -> bool {
        // Drain is complete if we're draining and no pending acks remain
        if self.draining_state == ControllerSenderDrainStatus::Completed
            || self.draining_state == ControllerSenderDrainStatus::Initiated
                && self.pending_replies.is_empty()
        {
            return true;
        }
        false
    }
}
