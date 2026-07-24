// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, time::Duration};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, Participant, ParticipantState, ProtoMessage as Message, ProtoName,
        ProtoSessionMessageType, ProtoSessionType,
    },
    messages::utils::{LEAVING_SESSION, TRUE_VAL},
};

use tracing::debug;

use crate::{
    common::{MessageDirection, SessionMessage, SessionOutput},
    errors::SessionError,
    mls_state::MlsState,
    persistence,
    runtime::maybe_await,
    session_controller::{PendingStatusUpdate, SessionControllerCommon, sign_control_messages},
    session_settings::SessionSettings,
    subscription_manager::{SubscriptionManager, SubscriptionOps},
    traits::{MessageHandler, ProcessingState},
};

pub struct SessionParticipant<P, V, I, M = SubscriptionManager>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<ProtoName>,

    /// list of participants
    group_list: HashMap<ProtoName, Participant>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// common session state
    common: SessionControllerCommon<P, V, M>,

    /// connection id from where the remote messages are received
    conn_id: Option<u64>,

    subscribed: bool,

    /// True while a LeaveCleanup self-message is pending (route teardown deferred).
    /// Prevents the processing loop from exiting before cleanup completes.
    pending_leave_cleanup: bool,

    /// inner layer
    inner: I,
}

impl<P, V, I, M> SessionParticipant<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    pub(crate) fn new(inner: I, settings: SessionSettings<P, V, M>) -> Self {
        let common = SessionControllerCommon::new(settings);

        SessionParticipant {
            moderator_name: None,
            group_list: HashMap::new(),
            mls_state: None,
            common,
            conn_id: None,
            subscribed: false,
            pending_leave_cleanup: false,
            inner,
        }
    }

    /// Construct a participant from restored state (already-loaded MLS group,
    /// moderator name and roster). `init()` will not overwrite `mls_state`. Call
    /// [`Self::restore_reconnect`] afterwards to re-establish routing.
    pub(crate) fn restore(
        inner: I,
        settings: SessionSettings<P, V, M>,
        mls_state: Option<MlsState<P, V>>,
        moderator_name: Option<ProtoName>,
        group_list: HashMap<ProtoName, Participant>,
    ) -> Self {
        let common = SessionControllerCommon::new(settings);

        SessionParticipant {
            moderator_name,
            group_list,
            mls_state,
            common,
            conn_id: None,
            subscribed: false,
            pending_leave_cleanup: false,
            inner,
        }
    }
}

/// Implementation of MessageHandler trait for SessionParticipant
/// This allows the participant to be used as a layer in the generic layer system
impl<P, V, I, M> MessageHandler for SessionParticipant<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    async fn init(&mut self) -> Result<(), SessionError> {
        // Initialize MLS
        // Skip when already populated: a restored participant carries a loaded
        // MLS group (see `restore`) that must not be overwritten.
        if self.mls_state.is_none() {
            self.mls_state = if let Some(mls_settings) = &self.common.settings.config.mls_settings {
                let mls = crate::mls_state::build_mls(
                    self.common.settings.identity_provider.clone(),
                    self.common.settings.identity_verifier.clone(),
                    self.common.settings.group_storage.clone(),
                )
                .with_enforce_pqc(self.common.settings.enforce_pqc);
                let mls_state =
                    MlsState::new(mls, mls_settings.header_integrity_validation_percent)
                        .await
                        .expect("failed to create MLS state");
                Some(mls_state)
            } else {
                None
            };
        }

        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<SessionOutput, SessionError> {
        let mut output = SessionOutput::new();

        match message {
            SessionMessage::OnMessage {
                mut message,
                direction,
                ack_tx,
            } => {
                // If the participant is offline drop all messages except:
                // - UpdateParticipantState from the app (South) — the app wants to come back online
                // - GroupAck/GroupNack — responses to our own rejoin/close
                if !self.common.online {
                    let msg_type = message.get_session_message_type();
                    let allowed = (msg_type == ProtoSessionMessageType::UpdateParticipantState
                        && direction == MessageDirection::South)
                        || msg_type == ProtoSessionMessageType::GroupAck
                        || msg_type == ProtoSessionMessageType::GroupNack
                        || msg_type == ProtoSessionMessageType::RejoinReply;

                    if !allowed {
                        debug!(
                            name = %self.common.settings.source,
                            "participant is off-line, drop the message",
                        );
                        return Err(SessionError::ParticipantOffLine);
                    }
                }

                // Any incoming message from a remote participant is proof of liveness
                if direction == MessageDirection::North {
                    self.common.sender.notify_received_activity(&message);
                }

                if message.get_session_message_type().is_command_message() {
                    debug!(
                        message = ?message.get_session_message_type(),
                        source = %message.get_source(),
                        "received message",
                    );
                    output.extend(
                        self.process_control_message(direction, message, ack_tx)
                            .await?,
                    );
                } else {
                    // Sending a data message south counts as activity
                    if direction == MessageDirection::South {
                        self.common.sender.notify_sent_activity();
                    }

                    if direction == MessageDirection::North
                        && let Some(mls_state) = &mut self.mls_state
                    {
                        maybe_await!(mls_state.process_message(&mut message, direction))?;
                    }

                    let inner_output = self
                        .inner
                        .on_message(SessionMessage::OnMessage {
                            message,
                            direction,
                            ack_tx,
                        })
                        .await?;

                    output.extend(inner_output);
                }
            }
            SessionMessage::MessageError { error } => {
                output.extend(self.handle_message_error(error).await?);
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    let current_epoch = match &self.mls_state {
                        Some(mls_state) => mls_state.mls.get_epoch(),
                        None => None,
                    };
                    output.extend(self.common.sender.on_timer_timeout(
                        message_id,
                        message_type,
                        current_epoch,
                    )?);
                } else {
                    let inner_output = self
                        .inner
                        .on_message(SessionMessage::TimerTimeout {
                            message_id,
                            message_type,
                            name,
                            timeouts,
                        })
                        .await?;

                    output.extend(inner_output);
                }
            }
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    self.common.sender.on_failure(message_id, message_type);

                    if let Some(pending_task) = self.common.pending_status_update.take()
                        && pending_task.message_id == message_id
                    {
                        // the rejoin request did not succeed, notify the application with an error
                        if pending_task.message_type == ProtoSessionMessageType::RejoinRequest {
                            // If we get a timeout for the RejoinRequest, it means that the rekey did
                            // not work so we need to return an error to the application
                            if let Some(tx) = pending_task.ack_tx {
                                let _ = tx.send(Err(SessionError::RejoinFailed));
                            }
                        } else if pending_task.message_type
                            == ProtoSessionMessageType::UpdateParticipantState
                        {
                            // If this was rejoin (UpdateParticipantState + OnLine) timeout,
                            // treat as partial success: non-responders are offline, notify app OK
                            if pending_task.status == ParticipantState::Online {
                                if let Some(tx) = pending_task.ack_tx {
                                    let _ = tx.send(Ok(()));
                                }
                                self.common.online = true;
                                self.common.sender.restart_heartbeat();
                            } else {
                                // close timeout: not all participants acknowledged, the message.
                                // we can still consider the participant as offline and notify success
                                // to the application. the other participants will discover that this
                                // participant is offline using the heartbeat mechanism.
                                self.common.online = false;
                                self.common.sender.stop_heartbeat();
                                if let Some(tx) = pending_task.ack_tx {
                                    let _ = tx.send(Ok(()));
                                }
                            }
                        }
                    }
                } else {
                    output.extend(
                        self.inner
                            .on_message(SessionMessage::TimerFailure {
                                message_id,
                                message_type,
                                name,
                                timeouts,
                            })
                            .await?,
                    );
                }
            }
            SessionMessage::StartDrain {
                grace_period: duration,
            } => {
                debug!("received drain signal");
                let p = CommandPayload::builder().leave_request().as_content();
                if let Some(moderator) = &self.moderator_name {
                    let mut msg = self.common.create_control_message(
                        moderator,
                        ProtoSessionMessageType::LeaveRequest,
                        rand::random::<u32>(),
                        p,
                        false,
                    )?;
                    debug!("start drain and notify the moderator");
                    msg.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

                    self.disconnect_from_group().await?;

                    output.extend(self.common.sender.on_message(&msg)?);
                }

                self.common.processing_state = ProcessingState::Draining;
                output.extend(
                    self.inner
                        .on_message(SessionMessage::StartDrain {
                            grace_period: duration,
                        })
                        .await?,
                );
                self.common.sender.start_drain();
            }
            SessionMessage::ParticipantDisconnected { name } => {
                if let Some(participant_name) = name {
                    debug!(
                        %participant_name,
                        "participant detected offline via missed heartbeats, marking offline locally",
                    );

                    if let Some(entry) = self.group_list.get_mut(&participant_name) {
                        entry.status = ParticipantState::Offline as i32;
                    }

                    self.remove_endpoint(&participant_name);
                    debug!("participant {} is now offline", participant_name);
                }
            }
            SessionMessage::LeaveCleanup => {
                self.disconnect_from_group().await?;
                self.disconnect_from_moderator().await?;
                self.pending_leave_cleanup = false;
            }
            _ => {
                return Err(SessionError::SessionMessageInternalUnexpected(Box::new(
                    message,
                )));
            }
        }

        maybe_await!(self.encrypt_output(&mut output))?;

        Ok(output)
    }

    async fn add_endpoint(
        &mut self,
        endpoint: &Participant,
    ) -> Result<SessionOutput, SessionError> {
        self.common
            .sender
            .add_participant(endpoint.name.as_ref().unwrap());
        self.inner.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &ProtoName) {
        self.common.sender.remove_participant(endpoint);
        self.inner.remove_endpoint(endpoint);
    }

    fn needs_drain(&self) -> bool {
        self.pending_leave_cleanup
            || !self.common.sender.drain_completed()
            || self.inner.needs_drain()
    }

    fn processing_state(&self) -> ProcessingState {
        self.common.processing_state
    }

    fn participants_list(&self) -> Vec<(ProtoName, ParticipantState)> {
        self.group_list
            .iter()
            .map(|(name, entry)| {
                let status =
                    ParticipantState::try_from(entry.status).unwrap_or(ParticipantState::Online);
                (name.clone(), status)
            })
            .collect()
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Participant-specific cleanup
        self.subscribed = false;
        self.common.sender.close();

        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await
    }
}

impl<P, V, I, M> SessionParticipant<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    /// Snapshot the participant's session + MLS state to the persistence store
    /// so it can be restored after a restart. No-op when persistence is
    /// disabled; failures are logged, never propagated.
    fn persist_state(&self) {
        let Some(kv) = self.common.settings.kv_store.as_ref() else {
            return;
        };

        let (group_id, last_mls_msg_id) = match self.mls_state.as_ref() {
            Some(m) => (m.mls.get_group_id(), m.last_mls_msg_id),
            None => (None, 0),
        };

        // Serialize each member as a `Participant` (name + settings + status).
        let group_list = self
            .group_list
            .values()
            .map(persistence::encode_participant)
            .collect();

        let moderator_name = self.moderator_name.as_ref().map(persistence::encode_name);

        let record = persistence::new_record(
            self.common.settings.id,
            &self.common.settings.source,
            &self.common.settings.destination,
            &self.common.settings.control,
            self.common.settings.direction,
            &self.common.settings.config,
            group_id,
            last_mls_msg_id,
            self.conn_id,
            persistence::PersistedRole::Participant {
                moderator_name,
                group_list,
            },
        );

        if let Err(e) = record
            .to_bytes()
            .and_then(|bytes| Ok(kv.put(&persistence::session_key(record.session_id), &bytes)?))
        {
            tracing::error!(error = %e, session_id = self.common.settings.id, "failed to persist participant session state");
        }

        // Save the app identity (now carrying the MLS-installed keypair) so a
        // restart restores the exact identity this session was built with.
        persistence::persist_app_identity(kv, &self.common.settings.identity_provider);
    }

    #[maybe_async::maybe_async]
    async fn encrypt_output(&mut self, output: &mut SessionOutput) -> Result<(), SessionError> {
        let mut identity_provider = self.common.settings.identity_provider.clone();
        if let Some(mls_state) = &self.mls_state {
            identity_provider = mls_state.mls.identity_provider().clone();
        }
        crate::session_controller::SessionController::apply_identity_to_slim_output(
            output,
            &identity_provider,
        )?;
        if let Some(mls_state) = &mut self.mls_state {
            self.common
                .sign_control_messages(output, &identity_provider)?;
            mls_state.encrypt_output(output).await?;
        } else {
            // Discovery messages always need to be signed as MLS settings are not available for the
            // receiver at this point
            sign_control_messages(output, &identity_provider)?;
        }
        Ok(())
    }

    /// Helper method to handle MessageError
    /// Extracts context from the error and routes to appropriate handler
    async fn handle_message_error(
        &mut self,
        error: SessionError,
    ) -> Result<SessionOutput, SessionError> {
        let Some(session_ctx) = error.session_context() else {
            tracing::warn!("Received MessageError without session context");
            return self
                .inner
                .on_message(SessionMessage::MessageError { error })
                .await;
        };

        if error.is_command_message_error() {
            self.common.sender.on_failure(
                session_ctx.message_id,
                session_ctx.get_session_message_type(),
            );
            Ok(SessionOutput::new())
        } else {
            self.inner
                .on_message(SessionMessage::MessageError { error })
                .await
        }
    }

    async fn process_control_message(
        &mut self,
        direction: MessageDirection,
        message: Message,
        ack_tx: Option<tokio::sync::oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::JoinRequest => self.on_join_request(message).await,
            ProtoSessionMessageType::GroupWelcome => self.on_welcome(message).await,
            ProtoSessionMessageType::GroupUpdate => self.on_group_update_message(message).await,
            ProtoSessionMessageType::LeaveRequest | ProtoSessionMessageType::GroupClose => {
                self.on_leave_request(message).await
            }
            ProtoSessionMessageType::Heartbeat => self.on_heartbeat(message).await,
            ProtoSessionMessageType::LeaveReply => {
                // this message is received when the moderator ack the
                // reception of the leave request sent on Drain start
                // if the participant in not on drain state drop the message
                if self.common.processing_state == ProcessingState::Draining {
                    return self.common.sender.on_message(&message);
                }
                Ok(SessionOutput::new())
            }
            ProtoSessionMessageType::UpdateParticipantState => {
                debug!(
                    name = %message.get_source(),
                    id = %message.get_id(),
                    direction = ?direction,
                    "received update participant state message",
                );
                match direction {
                    MessageDirection::North => {
                        self.on_update_participant_state_from_slim(message).await
                    }
                    MessageDirection::South => {
                        self.on_update_participant_state_from_app(message, ack_tx)
                            .await
                    }
                }
            }
            ProtoSessionMessageType::GroupAck => {
                self.on_group_ack(&message)?;
                Ok(SessionOutput::new())
            }
            ProtoSessionMessageType::GroupNack => self.on_group_nack(&message).await,
            ProtoSessionMessageType::RejoinReply => self.on_rejoin_reply(&message).await,
            ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::RejoinRequest => {
                debug!(
                    control_message_type = ?message.get_session_message_type(),
                    "Unexpected control message type",
                );
                Ok(SessionOutput::new())
            }
            _ => {
                debug!(
                    message_type = ?message.get_session_message_type(),
                    "Unexpected message type",
                );
                Ok(SessionOutput::new())
            }
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received join request",
        );
        let source = msg.get_source();
        self.moderator_name = Some(source.clone());

        self.common
            .add_route(source.clone(), msg.get_incoming_conn())
            .await?;

        // setup the control sender with missing group name
        if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            self.common
                .sender
                .set_group_name(self.common.settings.control.clone());
        } else {
            // in point-to-point sessions the group name is the same as the destination
            self.common
                .sender
                .set_group_name(self.common.settings.destination.clone());
        }

        let key_package = if let Some(mls_state) = &mut self.mls_state {
            debug!("mls enabled, create the package key");
            let key = maybe_await!(mls_state.generate_key_package())?;
            Some(key)
        } else {
            None
        };

        let participant = Participant::new(
            self.common.settings.source.clone(),
            self.common.settings.direction.to_participant_settings(),
        );

        let content = CommandPayload::builder()
            .join_reply(key_package, participant)
            .as_content();

        debug!("send join reply message");
        let reply = self.common.create_control_message(
            &source,
            ProtoSessionMessageType::JoinReply,
            msg.get_id(),
            content,
            false,
        )?;

        Ok(SessionOutput::to_slim(reply))
    }

    async fn on_welcome(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received welcome message",
        );

        if let Some(mls_state) = &mut self.mls_state {
            maybe_await!(mls_state.process_welcome_message(&msg))?;
        }

        self.join(&msg).await?;

        let list = &msg
            .get_payload()
            .unwrap()
            .as_command_payload()?
            .as_welcome_payload()?
            .participants;
        for p in list {
            let name = p.get_name()?;
            self.group_list.insert(name.clone(), p.clone());

            if name != self.common.settings.source.clone() {
                debug!(name = %msg.get_source(), "add endpoint to the session");
                // add a route to the new endpoint, this is needed in case of message retransmission
                // skip the moderator as the route is already added in on_join_request
                if self.moderator_name.as_ref() != Some(&name) {
                    self.common
                        .add_route(name.clone(), msg.get_incoming_conn())
                        .await?;
                }
                if p.status == ParticipantState::Online as i32 {
                    self.add_endpoint(p).await?;
                }
            }
        }

        let ack = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;

        // Joined the group (roster + MLS established): persist for restore.
        self.persist_state();

        Ok(SessionOutput::to_slim(ack))
    }

    async fn on_group_update_message(
        &mut self,
        msg: Message,
    ) -> Result<SessionOutput, SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received update",
        );

        let update = msg
            .get_payload()
            .unwrap()
            .as_command_payload()?
            .as_group_update_payload()?
            .clone();

        let op = slim_datapath::api::GroupUpdateOp::try_from(update.op)
            .map_err(|_| SessionError::InvalidGroupUpdateOp(update.op))?;

        // Check if this is a Rejoin for ourselves — if so, skip MLS processing
        // (we already processed the welcome in on_rejoin_reply) but still refresh the group list
        let is_self_rejoin = op == slim_datapath::api::GroupUpdateOp::Rejoin
            && update
                .participant
                .as_ref()
                .and_then(|p| p.get_name().ok())
                .map(|name| name == self.common.settings.source)
                .unwrap_or(false);

        if !is_self_rejoin && let Some(mls_state) = &mut self.mls_state {
            debug!("process mls control update");
            let source_proto = self.common.settings.source.clone();
            let ret = maybe_await!(mls_state.process_control_message(msg.clone(), &source_proto))?;

            if !ret {
                debug!(
                    id = %msg.get_id(),
                    "Message already processed, drop it",
                );
                return Ok(SessionOutput::new());
            }
        }
        match op {
            slim_datapath::api::GroupUpdateOp::Add => {
                if let Some(ref new_participant) = update.participant {
                    let name = new_participant.get_name()?;
                    self.group_list
                        .insert(name.clone(), new_participant.clone());

                    debug!(name = %msg.get_source(), "add endpoint to session");
                    // add a route to the new endpoint, this is needed in case of message retransmission
                    self.common.add_route(name, msg.get_incoming_conn()).await?;
                    self.add_endpoint(new_participant).await?;
                }
            }
            slim_datapath::api::GroupUpdateOp::Remove => {
                if let Some(ref removed_participant) = update.participant {
                    let name = removed_participant.get_name()?;
                    self.group_list.remove(&name);

                    debug!(name = %msg.get_source(), "remove endpoint from session");
                    // Skip delete_route when the removed participant is ourselves: we never
                    // set up a recv_from subscription for our own name, so the datapath
                    // would return SubscriptionNotFound and block the GroupAck.
                    if name != self.common.settings.source.clone() {
                        self.common
                            .delete_route(name.clone(), msg.get_incoming_conn())
                            .await?;
                    }
                    self.inner.remove_endpoint(&name);
                }
            }
            slim_datapath::api::GroupUpdateOp::Rejoin => {
                if let Some(ref rejoined_participant) = update.participant {
                    let name = rejoined_participant.get_name()?;

                    if is_self_rejoin {
                        debug!("we are the rejoining participant, rebuild the group list");
                        // We are the rejoining participant — rebuild the group list
                        // from the participants field (members may have been added/removed
                        // while we were offline)
                        self.group_list.clear();
                        for p in &update.participants {
                            let p_name = p.get_name()?;
                            if p.status == ParticipantState::Offline as i32 {
                                self.remove_endpoint(&p_name);
                            } else if p_name != self.common.settings.source {
                                // Add endpoint for online participants (skip self)
                                self.add_endpoint(p).await?;
                            }
                            self.group_list.insert(p_name, p.clone());
                        }
                    } else {
                        debug!("participant rejoined: {}", name);
                        // Another participant rejoined — update their status
                        if let Some(entry) = self.group_list.get_mut(&name) {
                            entry.status = ParticipantState::Online as i32;
                        } else {
                            // Unknown participant — add them to the group
                            let mut updated = rejoined_participant.clone();
                            updated.status = ParticipantState::Online as i32;
                            self.group_list.insert(name.clone(), updated);
                            self.common.add_route(name, msg.get_incoming_conn()).await?;
                            self.add_endpoint(rejoined_participant).await?;
                        }
                    }
                }
            }
        }

        let msg = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;

        // Roster + MLS state advanced: persist so the session can be restored.
        self.persist_state();

        Ok(SessionOutput::to_slim(msg))
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!("close session");
        self.common.processing_state = ProcessingState::Draining;

        let (reply_type, reply_content) = match msg.get_session_message_type() {
            ProtoSessionMessageType::GroupClose => {
                // The group is being destroyed — no acks will ever arrive.
                // Close immediately so all pending CompletionHandles resolve
                // with SessionClosed rather than waiting for the retry timer (~9 s).
                self.on_shutdown().await?;
                self.common.sender.close();
                (
                    ProtoSessionMessageType::GroupAck,
                    CommandPayload::builder().group_ack().as_content(),
                )
            }
            _ => {
                // LeaveRequest: drain gracefully, waiting for in-flight acks.
                self.inner
                    .on_message(SessionMessage::StartDrain {
                        grace_period: Duration::from_secs(60), // not used in session
                    })
                    .await?;
                self.common.sender.start_drain();
                (
                    ProtoSessionMessageType::LeaveReply,
                    CommandPayload::builder().leave_reply().as_content(),
                )
            }
        };

        let reply = self.common.create_control_message(
            &msg.get_source(),
            reply_type,
            msg.get_id(),
            reply_content,
            false,
        )?;

        let output = SessionOutput::to_slim(reply);

        // Remove session from pool IMMEDIATELY so that new DiscoveryRequests
        // (e.g., re-adding this participant) are handled as fresh sessions.
        self.common
            .settings
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.settings.id,
            }))
            .await
            .map_err(|_e| SessionError::SessionDeleteMessageSendFailed)?;

        // Schedule disconnect cleanup for AFTER the LeaveReply is dispatched.
        self.pending_leave_cleanup = true;
        self.common
            .settings
            .tx_session
            .send(SessionMessage::LeaveCleanup)
            .await
            .map_err(|_| SessionError::SlimMessageSendFailed)?;

        Ok(output)
    }

    async fn on_heartbeat(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        let source = msg.get_source();
        debug!(from = %source, "received heartbeat");

        // Check if the source is in the group list
        let Some(entry) = self.group_list.get_mut(&source) else {
            debug!(from = %source, "dropping heartbeat from unknown participant");
            return Ok(SessionOutput::new());
        };

        // Extract heartbeat and check epoch regardless of participant state.
        // If remote epoch is higher than ours, we fell behind (e.g. brief network
        // disconnection) and need to rejoin.
        let heartbeat_payload = msg.extract_heartbeat()?;
        let current_epoch = self.mls_state.as_ref().and_then(|s| s.mls.get_epoch());

        if let Some(local_epoch) = current_epoch
            && heartbeat_payload.epoch > local_epoch
        {
            // Remote epoch is higher than ours — the group was updated while we were
            // offline (e.g. network disconnection). Send a rejoin request to the
            // moderator to get the latest group state.
            debug!(
                from = %source,
                heartbeat_epoch = heartbeat_payload.epoch,
                current_epoch = local_epoch,
                "epoch behind remote, sending rejoin request to moderator"
            );

            // Only send rejoin if we don't already have one pending
            if self.common.pending_status_update.is_none() {
                let key_package = if let Some(mls_state) = &mut self.mls_state {
                    maybe_await!(mls_state.generate_key_package())?
                } else {
                    vec![]
                };

                let moderator = self
                    .moderator_name
                    .clone()
                    .ok_or(SessionError::ModeratorNotFound)?;
                let msg_id = rand::random::<u32>();
                let participant = self.common.settings.source.clone();
                let rejoin_msg = self.common.create_control_message(
                    &moderator,
                    ProtoSessionMessageType::RejoinRequest,
                    msg_id,
                    CommandPayload::builder()
                        .rejoin_request(participant, key_package)
                        .as_content(),
                    false,
                )?;

                self.common.pending_status_update = Some(PendingStatusUpdate {
                    message_id: msg_id,
                    message_type: ProtoSessionMessageType::RejoinRequest,
                    status: ParticipantState::Online,
                    ack_tx: None,
                });

                debug!(
                    name = %moderator,
                    id = %msg_id,
                    "sent rejoin request to moderator (epoch mismatch on heartbeat)",
                );
                return self.common.sender.on_message(&rejoin_msg);
            }
            return Ok(SessionOutput::new());
        }

        if entry.status == ParticipantState::Online as i32 {
            debug!(
                "participant is online and epoch matches, forward heartbeat to sender for tracking"
            );
            // Participant is online and epoch matches, just forward to sender for tracking
            return self.common.sender.on_message(&msg);
        }

        // Participant is offline — check if MLS epoch matches
        let epoch_matches = match current_epoch {
            Some(epoch) => heartbeat_payload.epoch == epoch,
            None => true, // no MLS, always allow reconnection
        };

        // Received a heartbeat from an offline participant — force sending our
        // own heartbeat so the remote peer can detect us and bring us back online
        // in its group list. This also advertises our current MLS epoch, letting
        // the peer decide whether a rejoin is needed.
        self.common.sender.force_heartbeat();

        if epoch_matches {
            // Epoch matches — bring participant back online
            debug!(from = %source, "participant back online (epoch matches), re-adding endpoint");
            entry.status = ParticipantState::Online as i32;
            let participant = entry.clone();
            self.add_endpoint(&participant).await?;
            self.common.sender.on_message(&msg)
        } else {
            debug!(
                from = %source,
                heartbeat_epoch = heartbeat_payload.epoch,
                current_epoch = ?current_epoch,
                "dropping heartbeat from offline participant with lower epoch"
            );
            Ok(SessionOutput::new())
        }
    }

    async fn on_update_participant_state_from_slim(
        &mut self,
        message: Message,
    ) -> Result<SessionOutput, SessionError> {
        let payload = message
            .get_payload()
            .ok_or(SessionError::MissingPayload {
                context: "update_participant_state",
            })?
            .as_command_payload()?
            .as_update_participant_state_payload()?;

        let participant_name = payload
            .participant
            .as_ref()
            .ok_or(SessionError::MissingPayload {
                context: "update_participant_state: missing participant name",
            })?
            .clone();

        // filter all the messages where participant name is equal to self.common.settings.source
        if participant_name == self.common.settings.source {
            debug!(
                name = %participant_name,
                "ignoring our own participant state update",
            );
            return Ok(SessionOutput::new());
        }

        let new_state = ParticipantState::try_from(payload.new_state).map_err(|_| {
            SessionError::MissingPayload {
                context: "update_participant_state: invalid participant state",
            }
        })?;

        // The sender (moderator or peer) may have reconnected over a new
        // connection (e.g. after a restart + rejoin), so refresh the reverse
        // route before replying — otherwise our ACK/NACK takes the stale path and
        // never arrives, and the sender retries until it times out.
        if let Some(in_conn) = message.try_get_incoming_conn() {
            self.common.add_route(message.get_source(), in_conn).await?;
        }

        match new_state {
            ParticipantState::Offline => {
                // Participant went offline: update state, remove route
                if let Some(p) = self.group_list.get_mut(&participant_name) {
                    if p.status == ParticipantState::Offline as i32 {
                        debug!(
                            name = %participant_name,
                            "received offline state for participant already offline",
                        );
                        return Ok(SessionOutput::new());
                    }
                    p.status = ParticipantState::Offline as i32;
                } else {
                    debug!(
                        name = %participant_name,
                        "received offline state for unknown participant",
                    );
                    return Ok(SessionOutput::new());
                }

                self.remove_endpoint(&participant_name);
                debug!("participant {} is now offline", participant_name);
            }
            ParticipantState::Online => {
                // First check if the participant is in our group
                if !self.group_list.contains_key(&participant_name) {
                    debug!(
                        name = %participant_name,
                        "received online state for unknown participant",
                    );
                    return Ok(SessionOutput::new());
                }

                // Check epoch: if MLS is enabled, verify the epoch matches
                let current_epoch = match &self.mls_state {
                    Some(mls_state) => mls_state.mls.get_epoch(),
                    None => None,
                };

                if let Some(epoch) = current_epoch
                    && payload.epoch != epoch
                {
                    debug!(
                        name = %participant_name,
                        local_epoch = epoch,
                        remote_epoch = payload.epoch,
                        "epoch mismatch on rejoin, sending NACK",
                    );
                    let nack = self.common.create_control_message(
                        &message.get_source(),
                        ProtoSessionMessageType::GroupNack,
                        message.get_id(),
                        CommandPayload::builder().group_nack().as_content(),
                        false,
                    )?;
                    return Ok(SessionOutput::to_slim(nack));
                }

                // Epoch matches (or MLS not enabled): bring participant online
                let state = self.group_list.get_mut(&participant_name).unwrap();
                state.status = ParticipantState::Online as i32;
                debug!("participant {} is now online", participant_name);
                let participant = state.clone();
                self.add_endpoint(&participant).await?;
            }
        }

        // Send ACK back to the source
        let ack = self.common.create_control_message(
            &message.get_source(),
            ProtoSessionMessageType::GroupAck,
            message.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;

        Ok(SessionOutput::to_slim(ack))
    }

    async fn on_update_participant_state_from_app(
        &mut self,
        message: Message,
        ack_tx: Option<tokio::sync::oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        // we can have only one update state per time, so if the pending state is already set, we return an error
        if self.common.pending_status_update.is_some() {
            return Err(SessionError::StatusChangeInProgress);
        }

        let destination = self.common.settings.control.clone();
        let msg_id = message.get_id();
        let payload = message
            .get_payload()
            .ok_or(SessionError::MissingPayload {
                context: "update_participant_state_from_app",
            })?
            .clone();

        // store the ack_tx in the common sender so that it can be used when the ack is received
        let status = ParticipantState::try_from(
            payload
                .as_command_payload()?
                .as_update_participant_state_payload()?
                .new_state,
        )
        .map_err(|_| SessionError::MissingPayload {
            context: "update_participant_state: invalid state",
        })?;

        // On rejoin (OnLine): mark all other participants as offline.
        // They will be moved back online as their ACKs arrive.
        if status == ParticipantState::Online {
            for (_, entry) in self.group_list.iter_mut() {
                entry.status = ParticipantState::Offline as i32;
            }
        }

        // Fill in the actual MLS epoch before sending
        let current_epoch = match &self.mls_state {
            Some(mls_state) => mls_state.mls.get_epoch().unwrap_or(0),
            None => 0,
        };
        let payload = CommandPayload::builder()
            .update_participant_state(self.common.settings.source.clone(), status, current_epoch)
            .as_content();

        self.common.pending_status_update = Some(PendingStatusUpdate {
            message_id: message.get_id(),
            message_type: ProtoSessionMessageType::UpdateParticipantState,
            status,
            ack_tx,
        });

        self.common.send_control_message(
            &destination,
            ProtoSessionMessageType::UpdateParticipantState,
            msg_id,
            payload,
            None,
            true,
        )
    }

    async fn on_rejoin_reply(&mut self, message: &Message) -> Result<SessionOutput, SessionError> {
        debug!(
            name = %message.get_source(),
            id = %message.get_id(),
            "received rejoin reply",
        );

        self.common.sender.on_message(message)?;

        // Use the welcome message in the rejoin reply payload to update the local MLS state
        let payload = message.extract_rejoin_reply()?;
        let mls_success = if let Some(mls_state) = &mut self.mls_state {
            if !payload.mls_content.is_empty() {
                // Reset the MLS state by processing the welcome message
                match maybe_await!(mls_state.mls.process_welcome(&payload.mls_content)) {
                    Ok(group_id) => {
                        mls_state.group = group_id;
                        mls_state.last_mls_msg_id = payload.commit_id;
                        debug!(
                            commit_id = payload.commit_id,
                            "MLS state reset via rejoin welcome",
                        );
                        true
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to process rejoin welcome");
                        false
                    }
                }
            } else {
                // if we are here MLS must be set and so the payload
                false
            }
        } else {
            // if we are here MLS must be set and so the payload
            false
        };

        // Notify the application of the result and update local online state
        if let Some(pending_task) = self.common.pending_status_update.take() {
            if pending_task.message_id == message.get_id()
                && pending_task.status == ParticipantState::Online
            {
                if mls_success {
                    if let Some(tx) = pending_task.ack_tx {
                        let _ = tx.send(Ok(()));
                    }
                    self.common.online = true;
                    self.common.sender.restart_heartbeat();
                } else {
                    if let Some(tx) = pending_task.ack_tx {
                        let _ = tx.send(Err(SessionError::RejoinFailed));
                    }
                }
            } else {
                // not matching, put it back
                self.common.pending_status_update = Some(pending_task);
            }
        }

        debug!(
            name = %message.get_source(),
            id = %message.get_id(),
            "rejoin reply processed, send ack to moderator",
        );
        // Send a GroupAck back to the moderator
        let ack_msg = self.common.create_control_message(
            &message.get_source(),
            ProtoSessionMessageType::GroupAck,
            message.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;
        Ok(SessionOutput::to_slim(ack_msg))
    }

    fn on_group_ack(&mut self, message: &Message) -> Result<(), SessionError> {
        let id = message.get_id();
        let source = message.get_source();

        debug!(
            name = %source,
            id = %id,
            "received group ack message",
        );
        self.common.sender.on_message(message)?;

        // If we have a pending rejoin (Online), move the ACK sender back online
        if let Some(ref pending_task) = self.common.pending_status_update
            && pending_task.message_id == id
            && pending_task.status == ParticipantState::Online
            && let Some(entry) = self.group_list.get_mut(&source)
        {
            entry.status = ParticipantState::Online as i32;
            debug!("participant {} confirmed online via ACK", source);
        }

        // check if the task is still pending and if the message id matches the pending task
        if !self.common.sender.is_still_pending(id) && self.common.pending_status_update.is_some() {
            let pending_task = self.common.pending_status_update.take().unwrap();
            if pending_task.message_id == id {
                if let Some(tx) = pending_task.ack_tx {
                    let _ = tx.send(Ok(()));
                }
                // update the local online state
                match pending_task.status {
                    ParticipantState::Offline => {
                        self.common.online = false;
                        self.common.sender.stop_heartbeat();
                    }
                    ParticipantState::Online => {
                        self.common.online = true;
                        self.common.sender.restart_heartbeat();
                    }
                }
            }
        }

        Ok(())
    }

    async fn on_group_nack(&mut self, message: &Message) -> Result<SessionOutput, SessionError> {
        // this may happen when the participant tries to rejoin with an old MLS epoch
        let id = message.get_id();

        debug!(
            name = %message.get_source(),
            id = %id,
            "received group nack message",
        );

        // check that we have a pending status update with matching id and status == Online
        if let Some(pending_task) = self.common.pending_status_update.take() {
            if pending_task.message_id == id && pending_task.status == ParticipantState::Online {
                // remove the pending message from the sender
                self.common
                    .sender
                    .on_failure(id, ProtoSessionMessageType::UpdateParticipantState);

                // generate a fresh key package for the rejoin request
                let key_package = if let Some(mls_state) = &mut self.mls_state {
                    maybe_await!(mls_state.generate_key_package())?
                } else {
                    vec![]
                };

                // send a rejoin request to the moderator
                let moderator = self
                    .moderator_name
                    .clone()
                    .ok_or(SessionError::ModeratorNotFound)?;
                let msg_id = rand::random::<u32>();
                let participant = self.common.settings.source.clone();
                let rejoin_msg = self.common.create_control_message(
                    &moderator,
                    ProtoSessionMessageType::RejoinRequest,
                    msg_id,
                    CommandPayload::builder()
                        .rejoin_request(participant, key_package)
                        .as_content(),
                    false,
                )?;

                // keep the ack_tx for when RejoinReply arrives
                self.common.pending_status_update = Some(PendingStatusUpdate {
                    message_id: msg_id,
                    message_type: ProtoSessionMessageType::RejoinRequest,
                    status: ParticipantState::Online,
                    ack_tx: pending_task.ack_tx,
                });

                debug!(
                    name = %moderator,
                    id = %msg_id,
                    "sent rejoin request to moderator",
                );
                return self.common.sender.on_message(&rejoin_msg);
            } else {
                // not matching, put it back
                self.common.pending_status_update = Some(pending_task);
            }
        }

        Ok(SessionOutput::new())
    }

    /// Re-establish routing for a restored participant over the current
    /// connection `conn`, mirroring the routes/subscriptions that `join` and
    /// `on_welcome` set up. Does not touch MLS or the roster (already restored).
    pub(crate) async fn restore_reconnect(&mut self, conn: u64) -> Result<(), SessionError> {
        self.subscribed = true;
        self.conn_id = Some(conn);

        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        let destination = self.common.settings.destination.clone();
        let control = self.common.settings.control.clone();
        self.common.add_route(destination.clone(), conn).await?;
        self.common.add_subscription(destination, conn).await?;
        self.common.add_route(control.clone(), conn).await?;
        self.common.add_subscription(control, conn).await?;

        // For each other group member (moderator included), skipping self:
        // re-add its route AND re-register it with the session sender. Without
        // re-registering, the restored roster does not repopulate the sender's
        // endpoint list, so the participant's replies are buffered instead of
        // sent ("no remote endpoint connected to the session").
        let local = self.common.settings.source.clone();
        let participants: Vec<Participant> = self
            .group_list
            .iter()
            .filter(|(name, _)| **name != local)
            .map(|(_, p)| p.clone())
            .collect();
        for p in &participants {
            if let Ok(pname) = p.get_name() {
                self.common.add_route(pname, conn).await?;
            }
            self.add_endpoint(p).await?;
        }

        Ok(())
    }

    async fn join(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        self.conn_id = Some(msg.get_incoming_conn());

        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        let destination = self.common.settings.destination.clone();
        let control = self.common.settings.control.clone();
        debug!(
            "subscribe to channel {} and control {}",
            destination, control
        );
        self.common
            .add_route(destination.clone(), msg.get_incoming_conn())
            .await?;
        self.common
            .add_subscription(destination, msg.get_incoming_conn())
            .await?;
        self.common
            .add_route(control.clone(), msg.get_incoming_conn())
            .await?;
        self.common
            .add_subscription(control, msg.get_incoming_conn())
            .await
    }

    async fn disconnect_from_group(&mut self) -> Result<(), SessionError> {
        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        if let Some(conn_id) = self.conn_id {
            self.common
                .delete_route(self.common.settings.destination.clone(), conn_id)
                .await?;
            self.common
                .delete_subscription(self.common.settings.destination.clone(), conn_id)
                .await?;
            self.common
                .delete_route(self.common.settings.control.clone(), conn_id)
                .await?;
            self.common
                .delete_subscription(self.common.settings.control.clone(), conn_id)
                .await?;
        }

        // remove also all the routes to the other participants except the moderator
        // it will be removed in disconnect_from_moderator
        for (n, _) in self.group_list.iter() {
            if self.moderator_name.as_ref() != Some(n)
                && let Err(e) = self
                    .common
                    .delete_route(n.clone(), self.conn_id.unwrap())
                    .await
            {
                tracing::warn!(error = %e, name = %n, "error deleting route");
            }
        }

        Ok(())
    }

    async fn disconnect_from_moderator(&mut self) -> Result<(), SessionError> {
        if let Some(conn_id) = self.conn_id
            && let Err(e) = self
                .common
                .delete_route(self.moderator_name.as_ref().unwrap().clone(), conn_id)
                .await
        {
            tracing::warn!(error = %e, name = ?self.moderator_name, "error disconnecting from moderator");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Direction;
    use crate::common::OutboundMessage;
    use crate::session_config::SessionConfig;
    use crate::session_settings::SessionSettings;
    use crate::test_utils::{MockInnerHandler, MockTokenProvider, MockVerifier};
    use slim_datapath::Status;
    use slim_datapath::api::{CommandPayload, ParticipantSettings, ProtoSessionType};
    use tokio::sync::mpsc;

    // --- Test Helpers -----------------------------------------------------------------------

    /// Drives `fut` to completion while automatically resolving any subscription
    /// ACKs that arrive on `rx_slim` (simulating the SLIM datapath ACK response).
    async fn run_with_acks<F, T>(
        fut: F,
        rx_slim: &mut mpsc::Receiver<Result<Message, Status>>,
        sub_mgr: &crate::subscription_manager::SubscriptionManager,
    ) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let mut pinned = Box::pin(fut);
        loop {
            tokio::select! {
                res = &mut pinned => return res,
                msg = rx_slim.recv() => {
                    if let Some(Ok(msg)) = msg && let Some(ack_id) = msg.get_subscription_id() {
                        let ack = Message::builder().build_subscription_ack(ack_id, true, "");
                        sub_mgr.resolve_ack(ack.get_subscription_ack());
                    }
                }
            }
        }
    }

    fn make_name(parts: &[&str; 3]) -> ProtoName {
        ProtoName::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    fn make_proto_name(parts: &[&str; 3]) -> ProtoName {
        ProtoName::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    fn setup_participant(
        session_type: ProtoSessionType,
    ) -> (
        SessionParticipant<MockTokenProvider, MockVerifier, MockInnerHandler>,
        mpsc::Receiver<Result<Message, Status>>,
        mpsc::Receiver<Result<SessionMessage, SessionError>>,
        mpsc::Receiver<SessionMessage>,
    ) {
        let source = make_name(&["local", "participant", "v1"]);
        let (destination, control) = match session_type {
            ProtoSessionType::Multicast => (
                make_name(&["channel", "name", "v1"]).with_id(1),
                make_name(&["channel", "name", "v1"]).with_id(2),
            ),
            ProtoSessionType::PointToPoint => (
                make_name(&["remote", "participant", "v1"]).with_id(100),
                make_name(&["remote", "participant", "v1"]).with_id(100),
            ),
            _ => panic!("Unsupported session type for test setup"),
        };

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, rx_session) = mpsc::channel(16);
        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());
        let (tx_session_layer, rx_session_layer) = mpsc::channel(16);

        let config = SessionConfig {
            session_type,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: false,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source,
            destination,
            control,
            config,
            direction: Direction::Bidirectional,
            slim_tx: tx_slim,
            app_tx: tx_app,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
            subscription_manager,
            service_id: String::new(),
            max_seen_control_message_ids_size:
                crate::session_settings::DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let participant = SessionParticipant::new(inner, settings);

        (participant, rx_slim, rx_session_layer, rx_session)
    }

    #[tokio::test]
    async fn test_participant_new() {
        let (participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);

        assert!(participant.moderator_name.is_none());
        assert!(participant.group_list.is_empty());
        assert!(participant.mls_state.is_none());
        assert!(!participant.subscribed);
    }

    #[tokio::test]
    async fn test_participant_init() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);

        let result = participant.init().await;
        assert!(result.is_ok());
        assert!(participant.mls_state.is_none()); // MLS is disabled in test setup
    }

    #[tokio::test]
    async fn test_participant_on_join_request() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);

        let join_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        Some(3),
                        Some(std::time::Duration::from_secs(1)),
                        None,
                        None,
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_join_request(join_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have set moderator name
        assert_eq!(participant.moderator_name, Some(moderator));

        // Should have returned outbound messages (join reply)
        let output = result.unwrap();
        assert!(
            !output.is_empty(),
            "Should have sent messages including join reply"
        );
    }

    #[tokio::test]
    async fn test_participant_on_welcome_multicast() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let participant1_name = make_name(&["participant1", "app", "v1"]).with_id(401);
        let participant2_name = make_name(&["participant2", "app", "v1"]).with_id(402);
        let participant1 = Participant::new(
            participant1_name.clone(),
            ParticipantSettings::bidirectional(),
        );
        let participant2 = Participant::new(
            participant2_name.clone(),
            ParticipantSettings::bidirectional(),
        );

        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![participant1.clone(), participant2.clone()], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result =
            run_with_acks(participant.on_welcome(welcome_msg), &mut rx_slim, &sub_mgr).await;
        assert!(result.is_ok());

        // Should have subscribed
        assert!(participant.subscribed);

        // Should have added participants to group list
        assert_eq!(participant.group_list.len(), 2);

        // Should have added endpoints (excluding self)
        assert_eq!(participant.inner.get_endpoints_added_count().await, 2);
    }

    #[tokio::test]
    async fn test_participant_on_group_add_message() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let new_participant_name = make_name(&["new_participant", "app", "v1"]).with_id(500);
        let new_participant = Participant::new(
            new_participant_name.clone(),
            ParticipantSettings::bidirectional(),
        );

        let add_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupUpdate)
            .session_id(1)
            .message_id(300)
            .payload(
                CommandPayload::builder()
                    .group_update(
                        slim_datapath::api::GroupUpdateOp::Add,
                        new_participant.clone(),
                        vec![],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_group_update_message(add_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have added participant to group list
        assert!(participant.group_list.contains_key(&new_participant_name));

        // Should have added endpoint
        assert_eq!(participant.inner.get_endpoints_added_count().await, 1);

        // Should have sent group ack
        let output = result.unwrap();
        assert!(!output.is_empty(), "Should have sent group ack");
    }

    #[tokio::test]
    async fn test_participant_on_group_remove_message() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let removed_participant_name = make_name(&["removed", "app", "v1"]).with_id(500);
        let removed_participant = Participant::new(
            removed_participant_name.clone(),
            ParticipantSettings::bidirectional(),
        );
        participant.group_list.insert(
            removed_participant_name.clone(),
            removed_participant.clone(),
        );

        let remove_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupUpdate)
            .session_id(1)
            .message_id(400)
            .payload(
                CommandPayload::builder()
                    .group_update(
                        slim_datapath::api::GroupUpdateOp::Remove,
                        removed_participant,
                        vec![],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_group_update_message(remove_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have removed participant from group list
        assert!(
            !participant
                .group_list
                .contains_key(&removed_participant_name)
        );

        // Should have removed endpoint
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(participant.inner.get_endpoints_removed_count().await, 1);

        // Should have sent group ack
        let output = result.unwrap();
        assert!(!output.is_empty(), "Should have sent group ack");
    }

    #[tokio::test]
    async fn test_participant_on_leave_request() {
        let (mut participant, _rx_slim, mut rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let leave_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(500)
            .payload(CommandPayload::builder().leave_request().as_content())
            .build_publish()
            .unwrap();

        let result = participant.on_leave_request(leave_msg).await;
        assert!(result.is_ok());

        // Should have sent leave reply
        let output = result.unwrap();
        assert!(!output.is_empty());
        let msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            msg.get_session_header().session_message_type(),
            ProtoSessionMessageType::LeaveReply
        );

        // Should have sent delete session message
        let delete_msg = rx_session_layer.try_recv();
        assert!(delete_msg.is_ok());
        if let Ok(Ok(SessionMessage::DeleteSession { session_id })) = delete_msg {
            assert_eq!(session_id, 1);
        } else {
            panic!("Expected DeleteSession message");
        }
    }

    #[tokio::test]
    async fn test_participant_join_multicast() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(participant.join(&welcome_msg), &mut rx_slim, &sub_mgr).await;
        assert!(result.is_ok());
        assert!(participant.subscribed);
    }

    #[tokio::test]
    async fn test_participant_join_point_to_point() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::PointToPoint);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        Some(3),
                        Some(std::time::Duration::from_secs(1)),
                        None,
                        None,
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.join(&msg).await;
        assert!(result.is_ok());
        assert!(participant.subscribed);
        // P2P doesn't send subscribe message
    }

    #[tokio::test]
    async fn test_participant_join_idempotent() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();

        // First join — run_with_acks drains and ACKs all subscribe messages
        run_with_acks(participant.join(&msg), &mut rx_slim, &sub_mgr)
            .await
            .unwrap();

        // Second join should do nothing (already subscribed)
        participant.join(&msg).await.unwrap();
        let second_sub = rx_slim.try_recv();
        assert!(
            second_sub.is_err(),
            "Second join should not send any messages"
        );
    }

    #[tokio::test]
    async fn test_participant_application_message_forwarding() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let source = participant.common.settings.source.clone();
        let destination = participant.common.settings.destination.clone();

        let app_msg = Message::builder()
            .source(source)
            .destination(destination)
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(1)
            .message_id(100)
            .application_payload("application/octet-stream", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        let result = participant
            .on_message(SessionMessage::OnMessage {
                message: app_msg,
                direction: crate::MessageDirection::South,
                ack_tx: None,
            })
            .await;

        assert!(result.is_ok());

        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_timer_timeout_control_message() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::JoinRequest,
                name: None,
                timeouts: 1,
            })
            .await;

        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_participant_timer_timeout_app_message() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::Msg,
                name: None,
                timeouts: 1,
            })
            .await;

        assert!(result.is_ok());
        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_timer_failure_control_message() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerFailure {
                message_id: 100,
                message_type: ProtoSessionMessageType::JoinRequest,
                name: None,
                timeouts: 3,
            })
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_participant_timer_failure_app_message() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerFailure {
                message_id: 100,
                message_type: ProtoSessionMessageType::Msg,
                name: None,
                timeouts: 3,
            })
            .await;

        assert!(result.is_ok());
        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_add_and_remove_endpoint() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let endpoint_name = make_name(&["endpoint", "app", "v1"]);
        let endpoint =
            Participant::new(endpoint_name.clone(), ParticipantSettings::bidirectional());

        // Add endpoint
        let result = participant.add_endpoint(&endpoint).await;
        assert!(result.is_ok());
        assert_eq!(participant.inner.get_endpoints_added_count().await, 1);

        // Remove endpoint
        participant.remove_endpoint(&endpoint_name);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(participant.inner.get_endpoints_removed_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_on_shutdown() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let result = participant.on_shutdown().await;
        assert!(result.is_ok());
        assert!(!participant.subscribed);
    }

    #[tokio::test]
    async fn test_participant_unexpected_control_messages() {
        let (mut participant, _rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        // Test DiscoveryRequest (unexpected for participant)
        let discovery_msg = Message::builder()
            .source(make_proto_name(&["someone", "app", "v1"]).with_id(300))
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(1)
            .message_id(100)
            .payload(CommandPayload::builder().discovery_request().as_content())
            .build_publish()
            .unwrap();

        let result: Result<SessionOutput, SessionError> = participant
            .process_control_message(MessageDirection::South, discovery_msg, None)
            .await;
        assert!(result.is_ok()); // Should handle gracefully
    }

    #[tokio::test]
    async fn test_participant_leave_multicast_unsubscribes() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;
        participant.conn_id = Some(12345); // Set conn_id so disconnect_from_group sends messages

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let sub_mgr = participant.common.settings.subscription_manager.clone();

        // disconnect_from_group sends delete_route (ACK-awaiting) + unsubscribe (ACK-awaiting)
        let result =
            run_with_acks(participant.disconnect_from_group(), &mut rx_slim, &sub_mgr).await;
        assert!(result.is_ok());

        // disconnect_from_moderator sends delete_route (ACK-awaiting)
        let result = run_with_acks(
            participant.disconnect_from_moderator(),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());
    }

    // --- UpdateParticipantState Tests ---

    #[tokio::test]
    async fn test_update_participant_state_offline_from_slim() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Add a participant to the group
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        participant.group_list.insert(
            other.clone(),
            Participant::new(other.clone(), ParticipantSettings::bidirectional()),
        );

        // Receive an UpdateParticipantState(OffLine)
        let msg = Message::builder()
            .source(other.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(other.clone(), ParticipantState::Offline, 0)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::North, msg, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Participant should be offline
        assert_eq!(
            participant.group_list.get(&other).unwrap().status,
            ParticipantState::Offline as i32
        );

        // Should have sent an ACK
        let output = result.unwrap();
        assert!(!output.is_empty());
        let ack_msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            ack_msg.get_session_message_type(),
            ProtoSessionMessageType::GroupAck
        );
    }

    #[tokio::test]
    async fn test_update_participant_state_online_from_slim_epoch_match() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        // Add a participant that is offline
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut offline_participant =
            Participant::new(other.clone(), ParticipantSettings::bidirectional());
        offline_participant.status = ParticipantState::Offline as i32;
        participant
            .group_list
            .insert(other.clone(), offline_participant);

        // MLS is None in test setup, so epoch check is skipped (always matches)
        let msg = Message::builder()
            .source(other.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(101)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(other.clone(), ParticipantState::Online, 0)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::North, msg, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Participant should be online now
        assert_eq!(
            participant.group_list.get(&other).unwrap().status,
            ParticipantState::Online as i32
        );

        // Should have sent an ACK
        let output = result.unwrap();
        assert!(!output.is_empty());
        let ack_msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            ack_msg.get_session_message_type(),
            ProtoSessionMessageType::GroupAck
        );
    }

    #[tokio::test]
    async fn test_update_participant_state_from_app_close() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Add another participant
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        participant.group_list.insert(
            other.clone(),
            Participant::new(other.clone(), ParticipantSettings::bidirectional()),
        );
        participant.common.sender.add_participant(&other);

        let (ack_tx, mut ack_rx) = tokio::sync::oneshot::channel();

        let msg = Message::builder()
            .source(participant.common.settings.source.clone())
            .destination(participant.common.settings.control.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        participant.common.settings.source.clone(),
                        ParticipantState::Offline,
                        0,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::South, msg, Some(ack_tx)),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have a pending status update
        assert!(participant.common.pending_status_update.is_some());
        let pending = participant.common.pending_status_update.as_ref().unwrap();
        assert_eq!(pending.status, ParticipantState::Offline);

        // Participants should still be online (they change on ACK completion)
        assert_eq!(
            participant.group_list.get(&other).unwrap().status,
            ParticipantState::Online as i32
        );

        // ack_rx should not have been notified yet
        assert!(ack_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_update_participant_state_from_app_rejoin_and_nack() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Add participants
        let other1 = make_name(&["other1", "participant", "v1"]).with_id(500);
        let other2 = make_name(&["other2", "participant", "v1"]).with_id(600);
        participant.group_list.insert(
            other1.clone(),
            Participant::new(other1.clone(), ParticipantSettings::bidirectional()),
        );
        participant.group_list.insert(
            other2.clone(),
            Participant::new(other2.clone(), ParticipantSettings::bidirectional()),
        );
        participant.common.sender.add_participant(&other1);
        participant.common.sender.add_participant(&other2);

        let (ack_tx, mut ack_rx) = tokio::sync::oneshot::channel();

        // Send rejoin from app
        let msg = Message::builder()
            .source(participant.common.settings.source.clone())
            .destination(participant.common.settings.control.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(201)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        participant.common.settings.source.clone(),
                        ParticipantState::Online,
                        u64::MAX,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::South, msg, Some(ack_tx)),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // All participants should be marked offline after rejoin send
        assert_eq!(
            participant.group_list.get(&other1).unwrap().status,
            ParticipantState::Offline as i32
        );
        assert_eq!(
            participant.group_list.get(&other2).unwrap().status,
            ParticipantState::Offline as i32
        );

        // Should have a pending status update with OnLine
        assert!(participant.common.pending_status_update.is_some());
        let msg_id = participant
            .common
            .pending_status_update
            .as_ref()
            .unwrap()
            .message_id;

        // Now receive a NACK → should trigger a rejoin request (not immediate failure)
        let nack_msg = Message::builder()
            .source(other1.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupNack)
            .session_id(1)
            .message_id(msg_id)
            .payload(CommandPayload::builder().group_nack().as_content())
            .build_publish()
            .unwrap();

        let result = run_with_acks(
            participant.process_control_message(MessageDirection::North, nack_msg, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // The pending status update should now be a RejoinRequest (not cleared)
        assert!(participant.common.pending_status_update.is_some());
        let pending = participant.common.pending_status_update.as_ref().unwrap();
        assert_eq!(pending.message_type, ProtoSessionMessageType::RejoinRequest);
        assert_eq!(pending.status, ParticipantState::Online);

        // The ack_tx should still be held (not resolved yet — waiting for RejoinReply)
        assert!(ack_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_status_change_in_progress_guard() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        participant.group_list.insert(
            other.clone(),
            Participant::new(other.clone(), ParticipantSettings::bidirectional()),
        );
        participant.common.sender.add_participant(&other);

        // Set a pending status update
        participant.common.pending_status_update = Some(PendingStatusUpdate {
            message_id: 100,
            message_type: ProtoSessionMessageType::UpdateParticipantState,
            status: ParticipantState::Offline,
            ack_tx: None,
        });

        // Try another state change — should fail
        let msg = Message::builder()
            .source(participant.common.settings.source.clone())
            .destination(participant.common.settings.control.clone())
            .identity("")
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(201)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        participant.common.settings.source.clone(),
                        ParticipantState::Online,
                        0,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::South, msg, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(matches!(result, Err(SessionError::StatusChangeInProgress)));
    }

    // --- Rejoin Tests -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_on_rejoin_reply_sends_ack() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Set up a pending status update matching a RejoinRequest
        let (ack_tx, _ack_rx) = tokio::sync::oneshot::channel();
        let msg_id = 42u32;
        participant.common.pending_status_update = Some(PendingStatusUpdate {
            message_id: msg_id,
            message_type: ProtoSessionMessageType::RejoinRequest,
            status: ParticipantState::Online,
            ack_tx: Some(ack_tx),
        });

        // Receive a RejoinReply (no MLS in test — mls_state is None so it won't process welcome)
        let reply_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinReply)
            .session_id(1)
            .message_id(msg_id)
            .payload(
                CommandPayload::builder()
                    .rejoin_reply(1, vec![])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.process_control_message(MessageDirection::North, reply_msg, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have sent a GroupAck back
        let output = result.unwrap();
        assert!(!output.is_empty());
        let ack_msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            ack_msg.get_session_message_type(),
            ProtoSessionMessageType::GroupAck
        );
        assert_eq!(ack_msg.get_id(), msg_id);
    }

    #[tokio::test]
    async fn test_on_group_update_rejoin_self_refreshes_group_list() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Set up a stale group_list with an old participant
        let stale = make_name(&["stale", "participant", "v1"]).with_id(800);
        participant.group_list.insert(
            stale.clone(),
            Participant::new(stale.clone(), ParticipantSettings::bidirectional()),
        );

        // Fresh participants list from moderator
        let self_name = participant.common.settings.source.clone();
        let self_participant =
            Participant::new(self_name.clone(), ParticipantSettings::bidirectional());
        let new_other = make_name(&["new_other", "participant", "v1"]).with_id(900);
        let new_other_participant =
            Participant::new(new_other.clone(), ParticipantSettings::bidirectional());

        let update_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupUpdate)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .group_update(
                        slim_datapath::api::GroupUpdateOp::Rejoin,
                        self_participant.clone(),
                        vec![self_participant, new_other_participant],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_group_update_message(update_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // group_list should be refreshed — stale participant gone, new one present
        assert!(!participant.group_list.contains_key(&stale));
        assert!(participant.group_list.contains_key(&self_name));
        assert!(participant.group_list.contains_key(&new_other));
    }

    #[tokio::test]
    async fn test_on_group_update_rejoin_other_marks_online() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Add another participant that is offline
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut offline_participant =
            Participant::new(other.clone(), ParticipantSettings::bidirectional());
        offline_participant.status = ParticipantState::Offline as i32;
        participant
            .group_list
            .insert(other.clone(), offline_participant.clone());

        let update_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupUpdate)
            .session_id(1)
            .message_id(300)
            .payload(
                CommandPayload::builder()
                    .group_update(
                        slim_datapath::api::GroupUpdateOp::Rejoin,
                        offline_participant,
                        vec![],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_group_update_message(update_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Participant should be online now
        assert_eq!(
            participant.group_list.get(&other).unwrap().status,
            ParticipantState::Online as i32
        );
    }

    #[tokio::test]
    async fn test_on_group_update_rejoin_unknown_other_adds_to_group() {
        let (mut participant, mut rx_slim, _rx_session_layer, _rx_session) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // An unknown participant that is rejoining
        let unknown = make_name(&["unknown", "participant", "v1"]).with_id(999);
        let unknown_participant =
            Participant::new(unknown.clone(), ParticipantSettings::bidirectional());

        let update_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupUpdate)
            .session_id(1)
            .message_id(400)
            .payload(
                CommandPayload::builder()
                    .group_update(
                        slim_datapath::api::GroupUpdateOp::Rejoin,
                        unknown_participant,
                        vec![],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = participant.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            participant.on_group_update_message(update_msg),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Unknown participant should have been added to group_list as online
        assert!(participant.group_list.contains_key(&unknown));
        assert_eq!(
            participant.group_list.get(&unknown).unwrap().status,
            ParticipantState::Online as i32
        );

        // Should have added endpoint
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(participant.inner.get_endpoints_added_count().await, 1);
    }
}
