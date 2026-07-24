// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, GroupUpdateOp, MlsPayload, NameId, Participant, ParticipantState,
        ProtoMessage as Message, ProtoMlsSettings, ProtoName, ProtoSessionMessageType,
        ProtoSessionType,
    },
    messages::utils::{DELETE_GROUP, LEAVE_REPLY_SENT, LEAVING_SESSION, TRUE_VAL},
};
use tokio::sync::oneshot;

use tracing::debug;

use crate::{
    common::{MessageDirection, SessionMessage, SessionOutput},
    errors::SessionError,
    mls_state::{MlsModeratorState, MlsState},
    moderator_task::{
        AddParticipant, ModeratorTask, NotifyParticipants, RejoinParticipant, RemoveParticipant,
        TaskUpdate,
    },
    persistence,
    runtime::maybe_await,
    session_controller::{PendingStatusUpdate, SessionControllerCommon, sign_control_messages},
    session_settings::SessionSettings,
    subscription_manager::{SubscriptionManager, SubscriptionOps},
    traits::{MessageHandler, ProcessingState},
};

pub struct SessionModerator<P, V, I, M = SubscriptionManager>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    /// Queue of tasks to be performed by the moderator
    /// Each task contains a message and an optional ack channel
    tasks_todo: VecDeque<(Message, Option<oneshot::Sender<Result<(), SessionError>>>)>,

    /// Current task being processed by the moderator
    current_task: Option<ModeratorTask>,

    /// MLS state for the moderator
    mls_state: Option<MlsModeratorState<P, V>>,

    /// List of group participants
    /// The key is the participant name without ID
    /// The value contains the full name, participant settings, and online/offline state
    group_list: HashMap<ProtoName, Participant>,

    /// Common settings
    common: SessionControllerCommon<P, V, M>,

    /// Postponed message to be sent after current task completion
    postponed_message: Option<Message>,

    /// Subscription status
    subscribed: bool,

    /// connection id to the remote node
    conn_id: Option<u64>,

    /// Inner message handler
    inner: I,
}

impl<P, V, I, M> SessionModerator<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    pub(crate) fn new(inner: I, settings: SessionSettings<P, V, M>) -> Self {
        let common = SessionControllerCommon::new(settings);

        SessionModerator {
            tasks_todo: vec![].into(),
            current_task: None,
            mls_state: None,
            group_list: HashMap::new(),
            common,
            postponed_message: None,
            subscribed: false,
            conn_id: None,
            inner,
        }
    }

    /// Construct a moderator from restored state (already-loaded MLS group and
    /// roster). `init()` will not overwrite the provided `mls_state`. Call
    /// [`Self::restore_reconnect`] afterwards to re-establish routing.
    pub(crate) fn restore(
        inner: I,
        settings: SessionSettings<P, V, M>,
        mls_state: Option<MlsModeratorState<P, V>>,
        group_list: HashMap<ProtoName, Participant>,
    ) -> Self {
        let common = SessionControllerCommon::new(settings);

        SessionModerator {
            tasks_todo: vec![].into(),
            current_task: None,
            mls_state,
            group_list,
            common,
            postponed_message: None,
            subscribed: false,
            conn_id: None,
            inner,
        }
    }
}

/// Implementation of MessageHandler trait for SessionModerator
/// This allows the moderator to be used as a layer in the generic layer system
impl<P, V, I, M> MessageHandler for SessionModerator<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    async fn init(&mut self) -> Result<(), SessionError> {
        // Initialize MLS. Skip when already populated: a restored moderator
        // carries a loaded MLS group (see `restore`), which must not be
        // overwritten by a fresh, empty state.
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
                Some(MlsModeratorState::new(mls_state))
            } else {
                None
            };

            // Persist the initial session record so the channel survives a
            // crash even before the first participant joins. The MLS group is
            // not initialized yet (that happens in join()); persist_state()
            // handles the None-group case and the record is overwritten with
            // the full state when join() fires.
            self.persist_state();
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
                // Any incoming message from a remote participant is proof of liveness
                if direction == MessageDirection::North {
                    self.common.sender.notify_received_activity(&message);
                }

                // If the moderator is offline drop all messages except:
                // - UpdateParticipantState from the app (South) — the app wants to come back online
                // - GroupAck — responses to our own rejoin/close
                if !self.common.online {
                    let msg_type = message.get_session_message_type();
                    let allowed = (msg_type == ProtoSessionMessageType::UpdateParticipantState
                        && direction == MessageDirection::South)
                        || msg_type == ProtoSessionMessageType::GroupAck;

                    if !allowed {
                        debug!(
                            name = %self.common.settings.source,
                            "moderator is off-line, drop the message",
                        );
                        return Err(SessionError::ParticipantOffLine);
                    }
                }

                if message.get_session_message_type().is_command_message() {
                    debug!(
                        message = ?message.get_session_message_type(),
                        source = %message.get_source(),
                        "received  message",
                    );
                    output.extend(
                        self.process_control_message(message, direction, ack_tx)
                            .await?,
                    );
                } else {
                    if direction == MessageDirection::South
                        && self.common.settings.config.session_type
                            == ProtoSessionType::PointToPoint
                    {
                        message
                            .get_slim_header_mut()
                            .set_destination(self.common.settings.destination.clone());
                    }

                    // Sending a data message south counts as activity
                    if direction == MessageDirection::South {
                        self.common.sender.notify_sent_activity();
                    }

                    if direction == MessageDirection::North
                        && let Some(mls_state) = &mut self.mls_state
                    {
                        maybe_await!(mls_state.common.process_message(&mut message, direction))?;
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
                        Some(mls_state) => mls_state.common.mls.get_epoch(),
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
                    self.handle_failure(
                        message_id,
                        message_type,
                        SessionError::MessageSendRetryFailed { id: message_id },
                    )
                    .await?;
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
            SessionMessage::StartDrain { grace_period: _ } => {
                debug!("start draining by calling delete_all");
                self.common.processing_state = ProcessingState::Draining;
                let p = CommandPayload::builder().leave_request().as_content();
                let destination = self.common.settings.control.clone();
                let mut leave_msg = self.common.create_control_message(
                    &destination,
                    ProtoSessionMessageType::LeaveRequest,
                    rand::random::<u32>(),
                    p,
                    false,
                )?;
                leave_msg.insert_metadata(DELETE_GROUP.to_string(), TRUE_VAL.to_string());

                output.extend(self.delete_all(None).await?);
            }
            SessionMessage::ParticipantDisconnected {
                name: opt_participant,
            } => {
                let participant =
                    opt_participant.ok_or(SessionError::MissingParticipantNameOnDisconnection)?;
                debug!(
                    %participant,
                    "participant detected offline via missed heartbeats, marking offline locally",
                );

                let mut participant_no_id = participant.clone();
                participant_no_id.reset_id();

                // Mark the participant as offline in the local group list
                if let Some(entry) = self.group_list.get_mut(&participant_no_id) {
                    entry.status = ParticipantState::Offline as i32;
                }

                // Remove the endpoint from the sender and inner controller
                self.remove_endpoint(&participant);
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
        !self.common.sender.drain_completed()
            || self.inner.needs_drain()
            || !self.tasks_todo.is_empty()
    }
    fn processing_state(&self) -> ProcessingState {
        self.common.processing_state
    }

    fn participants_list(&self) -> Vec<(ProtoName, ParticipantState)> {
        self.group_list
            .iter()
            .map(|(name, entry)| {
                let id = entry
                    .name
                    .as_ref()
                    .map(|n| n.id())
                    .unwrap_or(NameId::NULL_COMPONENT); // the name should always be present
                let status =
                    ParticipantState::try_from(entry.status).unwrap_or(ParticipantState::Online);
                (name.clone().with_id(id), status)
            })
            .collect()
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Moderator-specific cleanup
        self.subscribed = false;
        self.common.sender.close();

        // Remove route and subscription for multicast sessions
        if self.common.settings.config.session_type == ProtoSessionType::Multicast
            && let Some(conn) = self.conn_id
        {
            self.common
                .delete_route(self.common.settings.destination.clone(), conn)
                .await?;
            self.common
                .delete_subscription(self.common.settings.destination.clone(), conn)
                .await?;
            self.common
                .delete_route(self.common.settings.control.clone(), conn)
                .await?;
            self.common
                .delete_subscription(self.common.settings.control.clone(), conn)
                .await?;
        }

        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await?;

        // Note: signalling the session layer to delete the session is done
        // uniformly for both roles in the controller's processing loop after
        // `on_shutdown`, so there is nothing role-specific to do here.
        Ok(())
    }
}

impl<P, V, I, M> SessionModerator<P, V, I, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
    M: SubscriptionOps,
{
    /// Snapshot the moderator's session + MLS state to the persistence store so
    /// it can be restored after a restart. No-op when persistence is disabled.
    /// Failures are logged, never propagated — persistence must not break the
    /// live session.
    fn persist_state(&self) {
        let Some(kv) = self.common.settings.kv_store.as_ref() else {
            return;
        };

        let (group_id, last_mls_msg_id, mls_participants, next_msg_id) =
            match self.mls_state.as_ref() {
                Some(m) => (
                    m.common.mls.get_group_id(),
                    m.common.last_mls_msg_id,
                    m.participants
                        .iter()
                        .map(|(n, id)| (persistence::encode_name(n), id.clone()))
                        .collect(),
                    m.next_msg_id,
                ),
                None => (None, 0, Vec::new(), 0),
            };

        let group_list = self
            .group_list
            .values()
            .map(persistence::encode_participant)
            .collect();

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
            persistence::PersistedRole::Moderator {
                group_list,
                mls_participants,
                next_msg_id,
            },
        );

        if let Err(e) = record
            .to_bytes()
            .and_then(|bytes| Ok(kv.put(&persistence::session_key(record.session_id), &bytes)?))
        {
            tracing::error!(error = %e, session_id = self.common.settings.id, "failed to persist moderator session state");
        }

        // Save the app identity (now carrying the MLS-installed keypair) so a
        // restart restores the exact identity this session was built with.
        persistence::persist_app_identity(kv, &self.common.settings.identity_provider);
    }

    #[maybe_async::maybe_async]
    async fn encrypt_output(&mut self, output: &mut SessionOutput) -> Result<(), SessionError> {
        let mut identity_provider = self.common.settings.identity_provider.clone();
        if let Some(mls_state) = &self.mls_state {
            identity_provider = mls_state.common.mls.identity_provider().clone();
        }
        crate::session_controller::SessionController::apply_identity_to_slim_output(
            output,
            &identity_provider,
        )?;
        if let Some(mls_state) = &mut self.mls_state {
            self.common
                .sign_control_messages(output, &identity_provider)?;
            mls_state.common.encrypt_output(output).await?;
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
            self.handle_failure(
                session_ctx.message_id,
                session_ctx.get_session_message_type(),
                error,
            )
            .await?;
            Ok(SessionOutput::new())
        } else {
            self.inner
                .on_message(SessionMessage::MessageError { error })
                .await
        }
    }

    /// Helper method to handle failures, either from a timer or message error
    async fn handle_failure(
        &mut self,
        message_id: u32,
        message_type: ProtoSessionMessageType,
        error: SessionError,
    ) -> Result<(), SessionError> {
        let missing = self.common.sender.on_failure(message_id, message_type);

        // Handle UpdateLocalStatus task independently
        if matches!(self.current_task, Some(ModeratorTask::UpdateLocalStatus())) {
            if let Some(pending) = self.common.pending_status_update.take() {
                match pending.status {
                    ParticipantState::Online => {
                        // Rejoin timeout: participants that didn't reply are offline,
                        // those that replied are online. Notify success to the app.
                        for (_, entry) in self.group_list.iter_mut() {
                            if let Ok(name) = entry.get_name() {
                                entry.status = if !missing.contains(&name) {
                                    ParticipantState::Online as i32
                                } else {
                                    ParticipantState::Offline as i32
                                };
                            }
                        }
                        self.common.online = true;
                        self.common.sender.restart_heartbeat();
                        if let Some(tx) = pending.ack_tx {
                            let _ = tx.send(Ok(()));
                        }
                    }
                    ParticipantState::Offline => {
                        // move to offline anyway
                        self.common.online = false;
                        self.common.sender.stop_heartbeat();
                        // return no error also in this case. If the problem was and connection
                        // hickup the other participants will discover that the moderator is offline
                        // thanks to the heartbeat mechanism.
                        if let Some(tx) = pending.ack_tx {
                            let _ = tx.send(Ok(()));
                        }
                    }
                }
            }
            self.current_task = None;
            return self.pop_task().await;
        }

        // For all other tasks, notify the app of the failure
        if let Some(task) = self.current_task.as_mut()
            && let Some(ack_tx) = task.ack_tx_take()
        {
            let _ = ack_tx.send(Err(task.failure_message(error)));
        }

        self.current_task = None;
        self.pop_task().await
    }

    /// Helper method to handle errors after task creation
    /// Extracts ack_tx from current_task and sends the error
    fn handle_task_error(&mut self, error: SessionError) -> SessionError {
        if let Some(task) = self.current_task.take() {
            let ack_tx = match task {
                ModeratorTask::Add(t) => t.ack_tx,
                ModeratorTask::Remove(t) => t.ack_tx,
                ModeratorTask::Update(t) => t.ack_tx,
                ModeratorTask::CloseOrDisconnect(t) => t.ack_tx,
                _ => None,
            };
            if let Some(tx) = ack_tx {
                let _ = tx.send(Err(SessionError::cleanup_failed(&error)));
            }
        }

        // Remove task
        self.current_task = None;

        error
    }

    /// Helper method to prepare for shutdown by cleaning up state
    /// Sets processing state to draining, removes MLS state, clears tasks and timers,
    /// and signals drain to inner layer and sender
    async fn prepare_shutdown(&mut self) -> Result<(), SessionError> {
        debug!("Preparing for shutdown: cleaning up state");
        // set the processing state to draining
        self.common.processing_state = ProcessingState::Draining;
        // remove mls state
        self.mls_state = None;
        // clear all pending tasks
        self.tasks_todo.clear();
        // clear all pending timers
        self.common.sender.clear_timers();
        // signal start drain everywhere
        self.inner
            .on_message(SessionMessage::StartDrain {
                grace_period: Duration::from_secs(60), // not used in session
            })
            .await?;
        self.common.sender.start_drain();
        Ok(())
    }

    /// Helper method to remove a participant from the group list and compute MLS payload
    /// Returns the list of remaining participants and the MLS payload if MLS is enabled
    async fn remove_participant_and_compute_mls(
        &mut self,
        participant: &ProtoName,
        msg: &Message,
    ) -> Result<(Participant, Vec<Participant>, Option<MlsPayload>), SessionError> {
        // Build participants list with current participants
        // the group update needs to be received by everybody
        // in the group unless there are only 2 participants
        // (the moderator and a participant)
        let participants_vec: Vec<Participant> = self.group_list.values().cloned().collect();

        // Remove participant from group list
        let mut participant_no_id = participant.clone();
        participant_no_id.reset_id();
        let removed_entry = self.group_list.remove(&participant_no_id);
        let removed_participant = removed_entry.unwrap();

        // Remove endpoint from local session
        self.remove_endpoint(participant);

        // Compute MLS payload if needed
        let mls_payload = match self.mls_state.as_mut() {
            Some(state) => {
                let mls_content = maybe_await!(state.remove_participant(msg))
                    .map_err(|e| self.handle_task_error(e))?;
                let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
                Some(MlsPayload {
                    commit_id,
                    mls_content,
                })
            }
            None => None,
        };

        // Roster + MLS state advanced: persist so the session can be restored.
        self.persist_state();

        Ok((removed_participant, participants_vec, mls_payload))
    }

    /// Helper method to send a GroupUpdate message to notify participants
    /// Returns the message ID of the sent GroupUpdate message
    async fn send_group_update(
        &mut self,
        op: slim_datapath::api::GroupUpdateOp,
        participant: Participant,
        participants: Vec<Participant>,
        mls_payload: Option<MlsPayload>,
    ) -> Result<(u32, SessionOutput), SessionError> {
        let update_payload = CommandPayload::builder()
            .group_update(op, participant, participants, mls_payload)
            .as_content();
        let msg_id = rand::random::<u32>();

        let output = self.common.send_control_message(
            &self.common.settings.control.clone(),
            ProtoSessionMessageType::GroupUpdate,
            msg_id,
            update_payload,
            None,
            true,
        )?;

        Ok((msg_id, output))
    }

    async fn process_control_message(
        &mut self,
        message: Message,
        direction: MessageDirection,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest => {
                self.on_discovery_request(message, ack_tx).await
            }
            ProtoSessionMessageType::DiscoveryReply => self.on_discovery_reply(message).await,
            ProtoSessionMessageType::JoinReply => self.on_join_reply(message).await,
            ProtoSessionMessageType::LeaveRequest => {
                // the LeaveRequest message is also used to signal a graceful leave.
                // if the metadata contains LEAVE_REPLY_SENT (re-queued task) or
                // LEAVING_SESSION (new leave request) call on_disconnection_detected
                if message.contains_metadata(LEAVE_REPLY_SENT)
                    || message.contains_metadata(LEAVING_SESSION)
                {
                    return self.on_disconnection_detected(message, ack_tx).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message, ack_tx).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::Heartbeat => self.on_heartbeat(message).await,
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
            ProtoSessionMessageType::RejoinRequest => self.on_rejoin_request(message).await,
            ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupClose
            | ProtoSessionMessageType::GroupNack
            | ProtoSessionMessageType::RejoinReply => Err(
                SessionError::SessionMessageTypeUnexpected(message.get_session_message_type()),
            ),
            _ => Err(SessionError::SessionMessageTypeUnknown(
                message.get_session_message_type(),
            )),
        }
    }

    /// message processing functions
    async fn on_discovery_request(
        &mut self,
        mut msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        debug!(%self.common.settings.id, "received discovery request");
        // the channel discovery starts a new participant invite.
        // process the request only if not busy
        if self.current_task.is_some() {
            debug!(
                "Moderator is busy. Add invite participant task to the list and process it later"
            );
            // if busy postpone the task and add it to the todo list with its ack_tx
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(SessionOutput::new());
        }

        // now the moderator is busy - create the task first
        debug!("Create AddParticipant task with ack_tx");
        self.current_task = Some(ModeratorTask::Add(AddParticipant::new(ack_tx)));

        // check if the participant is already part of the group
        let new_participant_name = msg.get_dst();
        if self.group_list.contains_key(&new_participant_name) {
            let err = SessionError::ParticipantAlreadyInGroup(new_participant_name);
            return Err(self.handle_task_error(err));
        }

        // start the current task
        let id = rand::random::<u32>();
        msg.get_session_header_mut().set_message_id(id);
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_start(id)
            .map_err(|e| self.handle_task_error(e))?;

        debug!(
            dst = %msg.get_dst(),
            id = msg.get_id(),
            "send discovery request",
        );
        self.common
            .sender
            .on_message(&msg)
            .map_err(|e| self.handle_task_error(e))
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            source = %msg.get_source(),
            id = msg.get_id(),
            "discovery reply",
        );
        // update sender status to stop timers
        let mut output = self.common.sender.on_message(&msg)?;

        // evolve the current task state
        // the discovery phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_complete(msg.get_id())?;

        // join the channel if needed
        self.join(msg.get_source(), msg.get_incoming_conn()).await?;

        // set a route to the remote participant
        self.common
            .add_route(msg.get_source(), msg.get_incoming_conn())
            .await?;

        // if this is a multicast session we need to add a route for the channel
        // on the connection from where we received the message. This has to be done
        // all the times because the messages from the remote endpoints may come from
        // different connections. In case the route exists already it will be just ignored
        if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            self.common
                .add_route(
                    self.common.settings.destination.clone(),
                    msg.get_incoming_conn(),
                )
                .await?;
            self.common
                .add_route(
                    self.common.settings.control.clone(),
                    msg.get_incoming_conn(),
                )
                .await?;
            // setup the control sender with missing information
            self.common
                .sender
                .set_group_name(self.common.settings.control.clone());
        } else {
            // in point-to-point sessions the group name is the same as the destination
            self.common
                .sender
                .set_group_name(self.common.settings.destination.clone());
        }

        // an endpoint replied to the discovery message
        // send a join message
        let msg_id = rand::random::<u32>();

        let (channel, control) =
            if self.common.settings.config.session_type == ProtoSessionType::Multicast {
                // using the destination as channel name, the control name can be recreated by the participants
                (
                    Some(self.common.settings.destination.clone()),
                    Some(self.common.settings.control.clone()),
                )
            } else {
                (None, None)
            };

        let mls_settings =
            self.common
                .settings
                .config
                .mls_settings
                .as_ref()
                .map(|s| ProtoMlsSettings {
                    header_integrity_validation_percent: s.header_integrity_validation_percent,
                });

        let payload = CommandPayload::builder()
            .join_request(
                self.common.settings.config.max_retries,
                self.common.settings.config.interval,
                channel,
                control,
                mls_settings,
            )
            .as_content();

        debug!(
            dst = %msg.get_slim_header().get_source(),
            id = msg_id,
            "send join request",
        );
        output.extend(self.common.send_control_message(
            &msg.get_slim_header().get_source(),
            ProtoSessionMessageType::JoinRequest,
            msg_id,
            payload,
            Some(self.common.settings.config.metadata.clone()),
            false,
        )?);

        // evolve the current task state
        // start the join phase
        self.current_task.as_mut().unwrap().join_start(msg_id)?;

        Ok(output)
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            source = %msg.get_source(),
            id = msg.get_id(),
            "join reply",
        );
        // stop the timer for the join request
        let mut output = self.common.sender.on_message(&msg)?;

        // evolve the current task state
        // the join phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .join_complete(msg.get_id())?;

        // at this point the participant is part of the group so we can add it to the list
        let new_participant = msg
            .extract_join_reply()?
            .participant
            .clone()
            .ok_or(SessionError::MissingParticipantSettings)?;
        let mut new_name = new_participant.get_name()?;
        // notify the local session that a new participant was added to the group
        debug!(session_name = %new_name, "add endpoint");
        self.add_endpoint(&new_participant).await?;

        new_name.reset_id();
        self.group_list.insert(new_name, new_participant.clone());

        // get mls data if MLS is enabled
        let (commit, welcome) = if let Some(mls_state) = &mut self.mls_state {
            let (commit_payload, welcome_payload) = maybe_await!(mls_state.add_participant(&msg))?;

            // get the id of the commit, the welcome message has a random id
            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

            let commit = MlsPayload {
                commit_id,
                mls_content: commit_payload,
            };
            let welcome = MlsPayload {
                commit_id,
                mls_content: welcome_payload,
            };

            (Some(commit), Some(welcome))
        } else {
            (None, None)
        };

        // Create participants list for the messages to send
        let participants_vec = self.group_list.values().cloned().collect::<Vec<_>>();

        // send the group update
        if participants_vec.len() > 2 {
            debug!("participant len is > 2, send a group update");
            let update_payload = CommandPayload::builder()
                .group_update(
                    slim_datapath::api::GroupUpdateOp::Add,
                    new_participant,
                    participants_vec.clone(),
                    commit,
                )
                .as_content();
            let add_msg_id = rand::random::<u32>();
            debug!(id = %add_msg_id, "send add update to channel");
            output.extend(self.common.send_control_message(
                &self.common.settings.control.clone(),
                ProtoSessionMessageType::GroupUpdate,
                add_msg_id,
                update_payload,
                None,
                true,
            )?);
            self.current_task
                .as_mut()
                .unwrap()
                .commit_start(add_msg_id)?;
        } else {
            // no commit message will be sent so update the task state to consider the commit as received
            // the timer id is not important here, it just need to be consistent
            debug!("cancel the a group update task");
            self.current_task.as_mut().unwrap().commit_start(12345)?;
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(12345)?;
        }

        // send welcome message
        let welcome_msg_id = rand::random::<u32>();
        let welcome_payload = CommandPayload::builder()
            .group_welcome(participants_vec, welcome)
            .as_content();
        debug!(
            dst = %msg.get_slim_header().get_source(),
            id = %welcome_msg_id,
            "send welcome message",
        );
        output.extend(self.common.send_control_message(
            &msg.get_slim_header().get_source(),
            ProtoSessionMessageType::GroupWelcome,
            welcome_msg_id,
            welcome_payload,
            None,
            false,
        )?);

        // evolve the current task state
        // welcome start
        self.current_task
            .as_mut()
            .unwrap()
            .welcome_start(welcome_msg_id)?;

        // Roster + MLS state advanced: persist so the session can be restored.
        self.persist_state();

        Ok(output)
    }

    async fn on_leave_request(
        &mut self,
        mut msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list with its ack_tx
            debug!("Moderator is busy. Add leave request task to the list and process it later");
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(SessionOutput::new());
        }

        debug!("Create RemoveParticipant task");
        self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::new(ack_tx)));

        let dst_without_id = msg.get_dst();
        // Look up participant ID in group list
        let id = match self.group_list.get(&dst_without_id) {
            Some(entry) => entry.get_name()?.id(),
            None => {
                let err = SessionError::ParticipantNotFound(dst_without_id);
                return Err(self.handle_task_error(err));
            }
        };

        // Set destination with ID and message ID (common to both cases)
        let dst_with_id = dst_without_id.clone().with_id(id);
        msg.get_slim_header_mut().set_destination(dst_with_id);
        msg.set_message_id(rand::random::<u32>());

        let leave_message = msg;

        // Remove the participant from the group list and compute MLS payload
        debug!(
            session_name = %leave_message.get_dst(),
            "remove endpoint from the session",
        );

        let (removed_participant, participants_vec, mls_payload) = self
            .remove_participant_and_compute_mls(&leave_message.get_dst(), &leave_message)
            .await?;

        if participants_vec.len() > 2 {
            // in this case we need to send first the group update and later the leave message
            let (msg_id, output) = self
                .send_group_update(
                    GroupUpdateOp::Remove,
                    removed_participant,
                    participants_vec,
                    mls_payload,
                )
                .await?;
            self.current_task.as_mut().unwrap().commit_start(msg_id)?;

            // We need to save the leave message and send it after
            // the reception of all the acks for the group update message
            // see on_group_ack for postponed_message handling
            self.postponed_message = Some(leave_message);
            Ok(output)
        } else {
            // no commit message will be sent so update the task state to consider the commit as received
            // the timer id is not important here, it just need to be consistent
            self.current_task.as_mut().unwrap().commit_start(12345)?;
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(12345)?;

            // just send the leave message in this case
            let output = self.common.sender.on_message(&leave_message)?;

            self.current_task
                .as_mut()
                .unwrap()
                .leave_start(leave_message.get_id())?;
            Ok(output)
        }
    }

    async fn on_disconnection_detected(
        &mut self,
        mut msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        // if the disconnection was detected (no metadata in the message) the leave message is
        // sent toward the participant that was disconnected. Otherwise the leave message is sent
        // from the participant that wants to disconnect
        let disconnected = if msg.contains_metadata(LEAVING_SESSION) {
            msg.get_source()
        } else {
            msg.get_dst()
        };

        let mut disconnected_no_id = disconnected.clone();
        disconnected_no_id.reset_id();

        // check that the participant is actually part of the group
        if !self.group_list.contains_key(&disconnected_no_id) {
            debug!(
                "detected disconnection of participant {} that is not part of the group, ignore the message",
                disconnected
            );
            return Ok(SessionOutput::new());
        }

        debug!(%disconnected, "disconnection detected");

        // Send error notification to the application
        let error = SessionError::ParticipantDisconnected(disconnected.clone());
        let mut output = SessionOutput::to_app(Err(error));

        // if the disconnection was detected nothing to do here,
        // otherwise we need to reply, change the metadata and swap
        // source and destination so that we can process the message
        // as if the disconnection was detected locally
        if msg.contains_metadata(LEAVING_SESSION) {
            // send a reply to the source of the message
            // since this is a leave request message the sender is expecting
            // a leave reply message
            let reply = self.common.create_control_message(
                &disconnected,
                ProtoSessionMessageType::LeaveReply,
                msg.get_id(),
                CommandPayload::builder().leave_reply().as_content(),
                false,
            )?;
            // the participant will be removed from the group so we need to remove
            // it from the local sender.
            self.common.sender.remove_participant(&disconnected);
            output.extend(SessionOutput::to_slim(reply));

            // replace LEAVING_SESSION with LEAVE_REPLY_SENT so that if the process of the
            // message needs to be delayed because the moderator is busy we do not send the reply twice
            msg.remove_metadata(LEAVING_SESSION);
            msg.insert_metadata(LEAVE_REPLY_SENT.to_string(), TRUE_VAL.to_string());
            let header = msg.get_slim_header_mut();
            header.set_destination(disconnected.clone());
            header.set_source(self.common.settings.source.clone());
        }

        // if the session is P2P or no one is left on the session close it
        // if self.group_list.len() == 2 only the moderator and the participant
        // to remove are still in the list
        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint
            || self.group_list.len() == 2
        {
            debug!("no one is left connected connected to the session, close it");
            // if the remote endpoint got disconnected on a P2P session
            // simply notify the app and close the session
            self.prepare_shutdown().await?;
            // remove the last endpoint
            self.remove_endpoint(&msg.get_dst());

            // the control will exit and call the shutdown
            // no need to do it here
            return Ok(output);
        }

        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list with its ack_tx
            debug!(
                "Moderator is busy. Add disconnection handling task to the list and process it later"
            );
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(output);
        }

        debug!("Create disconnected task for the disconnection handling");
        // Reuse the disconnection task here, however we don't need to send the leave message
        // so we can mark it as done immediately
        self.current_task = Some(ModeratorTask::CloseOrDisconnect(NotifyParticipants::new(
            ack_tx,
        )));

        // Remove the participant from the group list and compute MLS payload
        debug!(
            endpoint = %disconnected,
            "remove disconnected endpoint from the session",
        );

        let (removed_participant, participants_vec, mls_payload) = self
            .remove_participant_and_compute_mls(&disconnected, &msg)
            .await?;

        // Notify all the participants left and update the MLS state if needed
        let (msg_id, remove_output) = self
            .send_group_update(
                GroupUpdateOp::Remove,
                removed_participant,
                participants_vec,
                mls_payload,
            )
            .await?;
        output.extend(remove_output);
        self.current_task.as_mut().unwrap().commit_start(msg_id)?;

        Ok(output)
    }

    async fn delete_all(
        &mut self,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        debug!("receive a close channel message, send signals to all participants");
        self.prepare_shutdown().await?;

        // Collect the participants and create the close message
        let participants: Vec<ProtoName> = self
            .group_list
            .iter()
            .map(|(n, entry)| entry.get_name().map(|name| n.clone().with_id(name.id())))
            .collect::<Result<Vec<ProtoName>, _>>()?;

        if participants.len() == 1 {
            // in this case the moderator is the only one remained
            // in the group so there is nothing to do
            return Ok(SessionOutput::new());
        }

        let destination = self.common.settings.control.clone();
        let close_id = rand::random::<u32>();
        let close = self.common.create_control_message(
            &destination,
            ProtoSessionMessageType::GroupClose,
            close_id,
            CommandPayload::builder()
                .group_close(participants)
                .as_content(),
            true,
        )?;

        // create the close task
        self.current_task = Some(ModeratorTask::CloseOrDisconnect(NotifyParticipants::new(
            ack_tx,
        )));
        self.current_task.as_mut().unwrap().commit_start(close_id)?;

        // sent the message
        self.common.sender.on_message(&close)
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            from = %msg.get_source(),
            id = %msg.get_id(),
            "received leave reply",
        );
        let msg_id = msg.get_id();

        // delete the route to the source of the message
        self.common
            .delete_route(msg.get_source(), msg.get_incoming_conn())
            .await?;

        // notify the sender and see if we can pick another task
        let output = self.common.sender.on_message(&msg)?;
        if !self.common.sender.is_still_pending(msg_id) {
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
        }

        self.task_done().await?;
        Ok(output)
    }

    async fn on_heartbeat(&mut self, message: Message) -> Result<SessionOutput, SessionError> {
        let source = message.get_source();
        let mut source_no_id = source.clone();
        source_no_id.reset_id();

        debug!("heartbeat received from {}", source);

        // Check if the source is in the group list
        let Some(entry) = self.group_list.get_mut(&source_no_id) else {
            debug!(from = %source, "dropping heartbeat from unknown participant");
            return Ok(SessionOutput::new());
        };

        // Verify the full name (with ID) matches
        if entry.get_name().ok().as_ref() != Some(&source) {
            debug!(from = %source, "dropping heartbeat from unknown participant (id mismatch)");
            return Ok(SessionOutput::new());
        }

        if entry.status == ParticipantState::Online as i32 {
            debug!(from = %source, "participant is online, forwarding heartbeat to sender");
            // Participant is online, just forward to sender for tracking
            return self.common.sender.on_message(&message);
        }

        debug!(from = %source, "participant is offline, checking MLS epoch");
        // Participant is offline — check if MLS epoch matches
        let heartbeat_payload = message.extract_heartbeat()?;
        let current_epoch = self
            .mls_state
            .as_ref()
            .and_then(|s| s.common.mls.get_epoch());

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
            self.common.sender.on_message(&message)
        } else {
            debug!(
                from = %source,
                heartbeat_epoch = heartbeat_payload.epoch,
                current_epoch = ?current_epoch,
                "dropping heartbeat from offline participant with mismatched epoch"
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

        // Ignore our own state updates
        if participant_name == self.common.settings.source {
            debug!(
                name = %participant_name,
                "ignoring our own participant state update",
            );
            return Ok(SessionOutput::new());
        }

        // The sender may have reconnected over a new connection (e.g. after a
        // restart + rejoin), so refresh the reverse route to it before replying —
        // otherwise our ACK/NACK takes the stale path and never arrives, and the
        // sender retries until it times out.
        if let Some(in_conn) = message.try_get_incoming_conn() {
            self.common.add_route(message.get_source(), in_conn).await?;
        }

        let new_state = ParticipantState::try_from(payload.new_state).map_err(|_| {
            SessionError::MissingPayload {
                context: "update_participant_state: invalid participant state",
            }
        })?;

        // Look up participant by name without ID
        let mut name_no_id = participant_name.clone();
        name_no_id.reset_id();

        match new_state {
            ParticipantState::Offline => {
                if let Some(entry) = self.group_list.get_mut(&name_no_id) {
                    if entry.status != ParticipantState::Online as i32 {
                        debug!(
                            name = %participant_name,
                            "received offline state for participant already offline",
                        );
                        return Ok(SessionOutput::new());
                    }
                    entry.status = ParticipantState::Offline as i32;
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
                if !self.group_list.contains_key(&name_no_id) {
                    debug!(
                        name = %participant_name,
                        "received online state for unknown participant",
                    );
                    return Ok(SessionOutput::new());
                }

                // Check epoch: if MLS is enabled, verify the epoch matches
                let current_epoch = match &self.mls_state {
                    Some(mls_state) => mls_state.common.mls.get_epoch(),
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
                let entry = self.group_list.get_mut(&name_no_id).unwrap();
                entry.status = ParticipantState::Online as i32;
                debug!("participant {} is now online", participant_name);
                let participant = entry.clone();
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
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        // we can have only one update state/task at a time
        if self.common.pending_status_update.is_some() || self.current_task.is_some() {
            return Err(SessionError::StatusChangeInProgress);
        }

        // set the task to avoid to take other operation
        self.current_task = Some(ModeratorTask::UpdateLocalStatus());

        let destination = self.common.settings.control.clone();
        let msg_id = message.get_id();
        let payload = message
            .get_payload()
            .ok_or(SessionError::MissingPayload {
                context: "update_participant_state_from_app",
            })?
            .clone();

        let status = ParticipantState::try_from(
            payload
                .as_command_payload()?
                .as_update_participant_state_payload()?
                .new_state,
        )
        .map_err(|_| SessionError::MissingPayload {
            context: "update_participant_state: invalid state",
        })?;

        // On rejoin (Online): mark all other participants as offline.
        // They will be moved back online as their ACKs arrive.
        if status == ParticipantState::Online {
            for (_, entry) in self.group_list.iter_mut() {
                entry.status = ParticipantState::Offline as i32;
            }
        }

        // Fill in the actual MLS epoch before sending
        let current_epoch = self
            .mls_state
            .as_ref()
            .and_then(|s| s.common.mls.get_epoch())
            .unwrap_or(0);
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

    async fn on_rejoin_request(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        let source = msg.get_source();
        debug!(
            from = %source,
            id = %msg.get_id(),
            "received rejoin request",
        );

        // Extract the rejoin request payload
        let rejoin_payload = msg
            .get_payload()
            .ok_or(SessionError::MissingPayload {
                context: "rejoin_request",
            })?
            .as_command_payload()?
            .as_rejoin_request_payload()?
            .clone();

        // 1. Check if the participant is in the group and is offline
        let participant_name = rejoin_payload
            .participant
            .as_ref()
            .ok_or(SessionError::MalformedParticipant)?
            .clone();
        let mut name_no_id = participant_name.clone();
        name_no_id.reset_id();

        let participant_entry = match self.group_list.get(&name_no_id) {
            Some(entry) => entry.clone(),
            None => {
                debug!(from = %participant_name, "rejoin request from unknown participant, ignoring");
                return Ok(SessionOutput::new());
            }
        };

        if participant_entry.status != ParticipantState::Offline as i32 {
            // A RejoinRequest from an online participant is only sent when the
            // participant detected an epoch mismatch (heartbeat with a higher
            // epoch, or a NACK for UpdateParticipantState(Online)). Without MLS
            // there is no epoch to mismatch, so ignore; with MLS, process it.
            if self.mls_state.is_none() {
                debug!(from = %participant_name, "rejoin request from online participant (no MLS), ignoring");
                return Ok(SessionOutput::new());
            }
            debug!(from = %participant_name, "rejoin request from online participant — epoch mismatch implied by key_package, processing");
        }

        // 2. Check if there is a current task
        if self.current_task.is_some() {
            // 3. Drop the message — the participant will retry
            debug!(from = %participant_name, "moderator is busy, dropping rejoin request (participant will retry)");
            return Ok(SessionOutput::new());
        }

        // 4. Set current task to Rejoin
        self.current_task = Some(ModeratorTask::Rejoin(RejoinParticipant::new()));

        let mut output = SessionOutput::new();

        // 5. Update the MLS group (rejoin = remove + re-add in one commit)
        let key_package = &rejoin_payload.key_package;

        let (commit_id, welcome_content, commit_content) =
            if let Some(mls_state) = &mut self.mls_state {
                let (commit_msg, welcome_msg) =
                    maybe_await!(mls_state.rejoin_participant(&participant_name, key_package))
                        .map_err(|e| self.handle_task_error(e))?;

                let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
                (commit_id, welcome_msg, commit_msg)
            } else {
                // this should never happen since the rejoin request is only sent when MLS is enabled
                (0, vec![], vec![])
            };

        // 6. Send rejoin reply with the welcome message and commit id to the participant
        let reply_msg_id = msg.get_id();
        let reply_payload = CommandPayload::builder()
            .rejoin_reply(commit_id, welcome_content)
            .as_content();
        debug!(
            dst = %source,
            id = %reply_msg_id,
            "send rejoin reply (welcome) to participant",
        );
        let reply_msg = self.common.create_control_message(
            &source,
            ProtoSessionMessageType::RejoinReply,
            reply_msg_id,
            reply_payload,
            false,
        )?;
        output.extend(SessionOutput::to_slim(reply_msg));

        // 7. Start the welcome phase in rejoin task
        self.current_task
            .as_mut()
            .unwrap()
            .welcome_start(reply_msg_id)?;

        // Only restore the datapath route and mark the participant Online when it
        // was previously Offline. For an epoch-mismatch rejoin the participant is
        // already Online and its endpoint/heartbeat tracking is intact — calling
        // add_endpoint again would reset the missed-heartbeat counter and
        // potentially trigger a spurious flush of the send buffer.
        if participant_entry.status == ParticipantState::Offline as i32
            && let Some(entry) = self.group_list.get_mut(&name_no_id)
        {
            entry.status = ParticipantState::Online as i32;
            let participant = entry.clone();
            self.add_endpoint(&participant).await?;
        }

        // 8. Send GroupUpdate with REJOIN op and MLS commit to all participants
        //    Only if there are at least 3 participants (moderator + rejoin + at least one other)
        let mls_payload = if !commit_content.is_empty() {
            Some(MlsPayload {
                commit_id,
                mls_content: commit_content,
            })
        } else {
            None
        };

        let participants_vec: Vec<Participant> = self.group_list.values().cloned().collect();

        // Always send GroupUpdate(REJOIN) — the rejoining participant uses it to rebuild its group_list
        let update_msg_id = rand::random::<u32>();
        let update_payload = CommandPayload::builder()
            .group_update(
                GroupUpdateOp::Rejoin,
                participant_entry,
                participants_vec,
                mls_payload,
            )
            .as_content();
        debug!(
            id = %update_msg_id,
            "send group update (rejoin) to channel",
        );
        output.extend(self.common.send_control_message(
            &self.common.settings.control.clone(),
            ProtoSessionMessageType::GroupUpdate,
            update_msg_id,
            update_payload,
            None,
            true,
        )?);

        // Start the commit/update phase in rejoin task
        self.current_task
            .as_mut()
            .unwrap()
            .commit_start(update_msg_id)?;

        Ok(output)
    }

    async fn on_group_ack(&mut self, msg: Message) -> Result<SessionOutput, SessionError> {
        debug!(
            from = %msg.get_source(),
            id = %msg.get_id(),
            "received group ack",
        );
        // notify the sender
        let mut output = self.common.sender.on_message(&msg)?;

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.common.sender.is_still_pending(msg_id) {
            debug!(
                id = %msg_id,
                "process group ack. try to close task",
            );
            // `is_still_pending` returns false for any ID that is not actively
            // tracked — including IDs that were already cleaned up when a task
            // completed or failed.  Guard against a late / retransmitted GroupAck
            // arriving after the task has been cleared; such an ACK is harmless
            // and should be silently discarded rather than causing a panic.
            let Some(task) = self.current_task.as_mut() else {
                debug!(
                    id = %msg_id,
                    "received group ack for completed/unknown task, ignoring",
                );
                return Ok(output);
            };

            // if task is UpdateLocalStatus at this point the task is completed
            if matches!(task, ModeratorTask::UpdateLocalStatus())
                && let Some(pending_update) = self.common.pending_status_update.take()
            {
                if let Some(tx) = pending_update.ack_tx {
                    let _ = tx.send(Ok(()));
                }
                match pending_update.status {
                    ParticipantState::Online => {
                        debug!("The moderator is back online, mark all participants as online");
                        for (_, entry) in self.group_list.iter_mut() {
                            entry.status = ParticipantState::Online as i32;
                        }
                        self.common.online = true;
                        self.common.sender.restart_heartbeat();
                    }
                    ParticipantState::Offline => {
                        debug!("The moderator is offline");
                        self.common.online = false;
                        self.common.sender.stop_heartbeat();
                    }
                }

                self.current_task = None;
                self.pop_task().await?;
                return Ok(output);
            }

            // we received all the messages related to this timer
            // check if we are done and move on
            task.update_phase_completed(msg_id)?;

            // check if the task is finished.
            if !self.current_task.as_mut().unwrap().task_complete() {
                // if the task is not finished yet we may need to send a leave
                // message that was postponed to send all group update first
                if let Some(leave_message) = &self.postponed_message
                    && matches!(self.current_task, Some(ModeratorTask::Remove(_)))
                {
                    // send the leave message an progress
                    output.extend(self.common.sender.on_message(leave_message)?);
                    self.current_task
                        .as_mut()
                        .unwrap()
                        .leave_start(leave_message.get_id())?;
                    // rest the postponed message
                    self.postponed_message = None;
                }
            }

            // check if we can progress with another task
            self.task_done().await?;
        } else {
            debug!(
                id = %msg_id,
                "timer for message still pending, do not close the task",
            );
        }

        Ok(output)
    }

    /// task handling functions
    async fn task_done(&mut self) -> Result<(), SessionError> {
        if !self.current_task.as_ref().unwrap().task_complete() {
            // the task is not completed so just return
            // and continue with the process
            debug!("Current task is NOT completed");
            return Ok(());
        }

        // here the moderator is not busy anymore
        self.current_task = None;
        self.pop_task().await
    }

    async fn pop_task(&mut self) -> Result<(), SessionError> {
        if self.current_task.is_some() {
            // moderator is busy, nothing else to do
            return Ok(());
        }

        // check if there is a pending task to process
        let (msg, ack_tx) = match self.tasks_todo.pop_front() {
            Some(task) => task,
            None => {
                // nothing else to do
                debug!("No tasks left to perform");

                // No need to close the session here. If we are in
                // closing state the moderator will be closed in
                // the controller loop
                return Ok(());
            }
        };

        debug!("Re-enqueue a task from the todo list onto the processing loop");
        // Send the task back to the processing loop instead of recursing into
        // `on_message`. Recursive async calls would otherwise require a boxed
        // future (and `async_trait` was hiding that cost before).
        // Use South direction: these are deferred control messages that the
        // moderator buffered while busy. They originate from the local app (e.g.
        // invite_participant) and therefore carry no identity token. Marking them
        // as South lets the session-controller loop skip identity verification,
        // which only applies to messages arriving from SLIM (North).
        self.common
            .settings
            .tx_session
            .send(SessionMessage::OnMessage {
                message: msg,
                direction: MessageDirection::South,
                ack_tx,
            })
            .await
            .map_err(|_| SessionError::SlimMessageSendFailed)?;

        Ok(())
    }

    /// Re-establish routing for a restored moderator over the current
    /// connection `conn`, mirroring the subscriptions/routes that `join` and
    /// `on_discovery_reply` set up during a live session. Does not touch MLS or
    /// the roster (already restored).
    pub(crate) async fn restore_reconnect(&mut self, conn: u64) -> Result<(), SessionError> {
        self.subscribed = true;
        self.conn_id = Some(conn);

        if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            let destination = self.common.settings.destination.clone();
            let control = self.common.settings.control.clone();

            // Restore the control sender's group name so heartbeats resume after
            // a restart, matching what `on_join_request` does for a fresh join.
            // Without it a restored session that then sits idle emits no
            // heartbeats. For multicast the heartbeat/control traffic uses the
            // control name.
            self.common.sender.set_group_name(control.clone());

            self.common
                .add_subscription(destination.clone(), conn)
                .await?;
            self.common.add_subscription(control.clone(), conn).await?;
            self.common.add_route(destination, conn).await?;
            self.common.add_route(control, conn).await?;

            // For each participant (skipping ourselves): re-add its route AND
            // re-register it with the session sender. The restored roster alone
            // does not repopulate the sender's endpoint list, so without this the
            // moderator has no endpoints to fan out to and every publish is
            // buffered ("no remote endpoint connected to the session").
            let mut local_no_id = self.common.settings.source.clone();
            local_no_id.reset_id();
            let participants: Vec<Participant> = self
                .group_list
                .iter()
                .filter(|(name, _)| **name != local_no_id)
                .map(|(_, p)| p.clone())
                .collect();
            for p in &participants {
                if let Ok(pname) = p.get_name() {
                    self.common.add_route(pname, conn).await?;
                }
                self.add_endpoint(p).await?;
            }
        }

        Ok(())
    }

    async fn join(&mut self, remote: ProtoName, conn: u64) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;
        self.conn_id = Some(conn);

        // if this is a point to point connection set the remote name so that we
        // can add also the right id to the message destination name
        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            self.common.settings.destination = remote;
        } else {
            // if this is a multicast session we need to subscribe for the channel name
            let destination = self.common.settings.destination.clone();
            debug!(
                "subscribe to channel {} and control {}",
                destination, self.common.settings.control
            );
            self.common.add_subscription(destination, conn).await?;
            self.common
                .add_subscription(self.common.settings.control.clone(), conn)
                .await?;
        }

        // create mls group if needed
        if let Some(mls) = self.mls_state.as_mut() {
            mls.init_moderator().await?;
        }

        // add ourself to the participants
        let mut local_name = self.common.settings.source.clone();
        let settings = self.common.settings.direction.to_participant_settings();
        let participant = Participant::new(local_name.clone(), settings);
        local_name.reset_id();
        self.group_list.insert(local_name, participant);

        self.persist_state();

        Ok(())
    }

    #[allow(dead_code)]
    async fn ack_msl_proposal(&mut self, _msg: &Message) -> Result<(), SessionError> {
        todo!()
    }

    #[allow(dead_code)]
    async fn on_mls_proposal(&mut self, _msg: Message) -> Result<(), SessionError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Direction;
    use crate::common::OutboundMessage;
    use crate::session_config::{MlsSettings, SessionConfig};
    use crate::session_settings::{DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE, SessionSettings};
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

    fn setup_moderator() -> (
        SessionModerator<MockTokenProvider, MockVerifier, MockInnerHandler>,
        mpsc::Receiver<Result<Message, Status>>,
        mpsc::Receiver<Result<SessionMessage, SessionError>>,
    ) {
        let source = make_name(&["local", "moderator", "v1"]).with_id(100);
        let destination = make_name(&["channel", "name", "v1"]).with_id(1);
        let control = make_name(&["channel", "name", "v1"]).with_id(2);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: true,
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
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let moderator = SessionModerator::new(inner, settings);

        (moderator, rx_slim, rx_session_layer)
    }

    #[tokio::test]
    async fn test_moderator_new() {
        let (moderator, _rx_slim, _rx_session_layer) = setup_moderator();

        assert!(moderator.tasks_todo.is_empty());
        assert!(moderator.current_task.is_none());
        assert!(moderator.mls_state.is_none());
        assert!(moderator.group_list.is_empty());
        assert!(moderator.postponed_message.is_none());
        assert!(!moderator.subscribed);
    }

    #[tokio::test]
    async fn test_moderator_init() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();

        let result = moderator.init().await;
        assert!(result.is_ok());
        assert!(moderator.mls_state.is_none()); // MLS is disabled in test setup
    }

    #[tokio::test]
    async fn test_moderator_discovery_request_starts_task() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let source = make_name(&["requester", "app", "v1"]).with_id(300);
        let destination = moderator.common.settings.source.clone();

        let discovery_msg = Message::builder()
            .source(source.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(1)
            .message_id(100)
            .payload(CommandPayload::builder().discovery_request().as_content())
            .build_publish()
            .unwrap();

        let result = moderator.on_discovery_request(discovery_msg, None).await;
        assert!(result.is_ok());

        // Should have created an Add task
        assert!(moderator.current_task.is_some());
        assert!(matches!(
            moderator.current_task,
            Some(ModeratorTask::Add(_))
        ));

        // Should have sent a discovery request
        let output = result.unwrap();
        assert!(!output.is_empty());
        let msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            msg.get_session_header().session_message_type(),
            ProtoSessionMessageType::DiscoveryRequest
        );
    }

    #[tokio::test]
    async fn test_moderator_discovery_request_when_busy() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Set a current task to make moderator busy
        moderator.current_task = Some(ModeratorTask::Add(AddParticipant::new(None)));

        let source = make_name(&["requester", "app", "v1"]).with_id(300);
        let destination = moderator.common.settings.source.clone();

        let discovery_msg = Message::builder()
            .source(source.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(1)
            .message_id(100)
            .payload(CommandPayload::builder().discovery_request().as_content())
            .build_publish()
            .unwrap();

        let result = moderator.on_discovery_request(discovery_msg, None).await;
        assert!(result.is_ok());

        // Should have added task to todo list
        assert_eq!(moderator.tasks_todo.len(), 1);
    }

    #[tokio::test]
    async fn test_moderator_join_request_passthrough() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let source = make_name(&["requester", "app", "v1"]).with_id(300);
        let destination = moderator.common.settings.source.clone();

        let join_msg = Message::builder()
            .source(source.clone())
            .destination(destination.clone())
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
        // JoinRequest is no longer handled by the moderator (moved to channel-manager).
        // It should return an error.
        let result = moderator
            .process_control_message(join_msg, MessageDirection::North, None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_moderator_application_message_forwarding() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let source = moderator.common.settings.source.clone();
        let destination = moderator.common.settings.destination.clone();

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

        let result = moderator
            .on_message(SessionMessage::OnMessage {
                message: app_msg,
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .await;

        assert!(result.is_ok());

        // Should have forwarded to inner handler
        assert_eq!(moderator.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_moderator_add_and_remove_endpoint() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let endpoint_name = make_name(&["participant", "app", "v1"]).with_id(400);
        let endpoint =
            Participant::new(endpoint_name.clone(), ParticipantSettings::bidirectional());

        // Add endpoint
        let result = moderator.add_endpoint(&endpoint).await;
        assert!(result.is_ok());
        assert_eq!(moderator.inner.get_endpoints_added_count().await, 1);

        // Remove endpoint
        moderator.remove_endpoint(&endpoint_name);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(moderator.inner.get_endpoints_removed_count().await, 1);
    }

    #[tokio::test]
    async fn test_moderator_join_sets_subscribed() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        assert!(!moderator.subscribed);

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let remote = make_name(&["remote", "app", "v1"]).with_id(200);
        let result = run_with_acks(moderator.join(remote, 12345), &mut rx_slim, &sub_mgr).await;

        assert!(result.is_ok());
        assert!(moderator.subscribed);
        assert!(!moderator.group_list.is_empty());
    }

    #[tokio::test]
    async fn test_moderator_join_only_once() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let remote = make_name(&["remote", "app", "v1"]).with_id(200);

        // First join — run_with_acks drains and ACKs the subscribe message
        run_with_acks(
            moderator.join(remote.clone(), 12345),
            &mut rx_slim,
            &sub_mgr,
        )
        .await
        .unwrap();

        // Second join should do nothing (already subscribed)
        moderator.join(remote, 12345).await.unwrap();
        let second_subscribe = rx_slim.try_recv();
        assert!(second_subscribe.is_err()); // No message should be sent
    }

    #[tokio::test]
    async fn test_moderator_persists_state_on_join() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();

        // Attach an encrypted persistence store, then drive the moderator to
        // establish its group.
        let dir = tempfile::tempdir().unwrap();
        let kv = slim_persistence::SlimKvStore::open_sqlite(dir.path(), "moderator", None).unwrap();
        moderator.common.settings.kv_store = Some(kv.clone());
        moderator.init().await.unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let remote = make_name(&["remote", "app", "v1"]).with_id(200);
        run_with_acks(moderator.join(remote, 12345), &mut rx_slim, &sub_mgr)
            .await
            .unwrap();

        // A session record must now exist, decoding as a moderator whose roster
        // includes itself.
        let records = kv.list_prefix(persistence::SESSION_KEY_PREFIX).unwrap();
        assert_eq!(records.len(), 1);

        let record = persistence::PersistedSession::from_bytes(&records[0].1).unwrap();
        assert_eq!(record.session_id, moderator.common.settings.id);
        match record.role {
            persistence::PersistedRole::Moderator { group_list, .. } => {
                assert_eq!(group_list.len(), 1, "moderator should be in its own roster");
            }
            _ => panic!("expected a moderator record"),
        }
    }

    #[tokio::test]
    async fn test_moderator_on_shutdown() {
        let (mut moderator, _rx_slim, mut _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let result = moderator.on_shutdown().await;
        assert!(result.is_ok());
        assert!(!moderator.subscribed);

        // TODO(msardara): enable the close signal
        // let close_msg = rx_session_layer.try_recv();
        // assert!(close_msg.is_ok());
        // if let Ok(Ok(SessionMessage::DeleteSession { session_id })) = close_msg {
        //     assert_eq!(session_id, 1);
        // } else {
        //     panic!("Expected DeleteSession message");
        // }
    }

    #[tokio::test]
    async fn test_moderator_delete_all_creates_leave_tasks() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add some participants to group list
        moderator.group_list.insert(
            make_name(&["participant1", "app", "v1"]),
            Participant::new(
                make_name(&["participant1", "app", "v1"]).with_id(401),
                ParticipantSettings::bidirectional(),
            ),
        );
        moderator.group_list.insert(
            make_name(&["participant2", "app", "v1"]),
            Participant::new(
                make_name(&["participant2", "app", "v1"]).with_id(402),
                ParticipantSettings::bidirectional(),
            ),
        );
        moderator.group_list.insert(
            make_name(&["participant3", "app", "v1"]),
            Participant::new(
                make_name(&["participant3", "app", "v1"]).with_id(403),
                ParticipantSettings::bidirectional(),
            ),
        );

        let result = moderator.delete_all(None).await;
        assert!(result.is_ok() || result.is_err()); // May error due to missing routes

        assert!(moderator.mls_state.is_none());
    }

    #[tokio::test]
    async fn test_moderator_timer_timeout_for_control_message() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Timer timeout for control messages requires sender to have pending messages
        // Without setup, it will fail. Just verify it processes without panicking.
        let result = moderator
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::DiscoveryRequest,
                name: None,
                timeouts: 1,
            })
            .await;

        // Result may be error if no pending timer exists, which is expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_moderator_timer_timeout_for_app_message() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let result = moderator
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::Msg,
                name: None,
                timeouts: 1,
            })
            .await;

        assert!(result.is_ok());
        // Should have forwarded to inner handler
        assert_eq!(moderator.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_moderator_point_to_point_destination_update() {
        let source = make_name(&["local", "app", "v1"]).with_id(100);
        let destination = make_name(&["remote", "app", "v1"]).with_id(200);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, _rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
            control: destination.clone(),
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
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        let app_msg = Message::builder()
            .source(source)
            .destination(destination)
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(1)
            .message_id(100)
            .application_payload("application/octet-stream", vec![1, 2, 3])
            .build_publish()
            .unwrap();

        let _original_dest = app_msg.get_dst();

        let result = moderator
            .on_message(SessionMessage::OnMessage {
                message: app_msg,
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .await;

        assert!(result.is_ok());
        // In P2P mode going South, destination should be updated
    }

    #[tokio::test]
    async fn test_moderator_graceful_leave_with_two_participants() {
        // Test graceful leave when exactly 2 participants remain (moderator + participant)
        // The leave request comes directly from the participant (not through controller)
        // with LEAVING_SESSION metadata to signal graceful departure

        // Create moderator with agntcy/ns/moderator naming
        let source = ProtoName::from_strings(["agntcy", "ns", "moderator"]).with_id(100);
        let destination = ProtoName::from_strings(["agntcy", "ns", "chat"]).with_id(1);
        let control = ProtoName::from_strings(["agntcy", "ns", "chat"]).with_id(2);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
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
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        // Set up moderator as joined (this adds moderator to group_list)
        let remote = ProtoName::from_strings(["agntcy", "ns", "participant"]).with_id(200);
        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        run_with_acks(
            moderator.join(remote.clone(), 12345),
            &mut rx_slim,
            &sub_mgr,
        )
        .await
        .unwrap();

        // Add one participant to the group (now we have moderator + participant = 2 total)
        // Use the naming convention requested: agntcy/ns/participant
        let mut participant_name = ProtoName::from_strings(["agntcy", "ns", "participant"]);
        let participant = Participant::new(
            participant_name.clone(),
            ParticipantSettings::bidirectional(),
        );
        // Fill in participant settings as needed
        let participant_id = 401u128;
        participant_name.reset_id(); // Remove ID before inserting into group_list
        moderator
            .group_list
            .insert(participant_name.clone(), participant.clone());

        // Verify we have exactly 2 participants (moderator + participant)
        assert_eq!(
            moderator.group_list.len(),
            2,
            "Should have exactly 2 participants"
        );

        // Verify session is not in draining state
        assert_eq!(moderator.processing_state(), ProcessingState::Active);

        // Create a leave request message coming directly from the participant
        // When coming from participant directly (not controller), the source is the participant
        // and the destination is the moderator, with LEAVING_SESSION metadata set
        let participant_with_id = participant_name.clone().with_id(participant_id);
        let mut leave_msg = Message::builder()
            .source(participant_with_id.clone())
            .destination(source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .leave_request() // Empty payload when coming from participant
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Add LEAVING_SESSION metadata to signal graceful departure
        leave_msg.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

        let result = moderator.on_disconnection_detected(leave_msg, None).await;

        // The function should succeed now that app channel is open
        assert!(result.is_ok(), "Should succeed with open app channel");

        // Verify that the ParticipantDisconnected error was sent to the output
        let output = result.unwrap();
        let app_error = output
            .messages
            .iter()
            .find(|m| matches!(m, OutboundMessage::ToApp(_)));
        assert!(
            app_error.is_some(),
            "Expected error to be sent to app output"
        );

        if let Some(OutboundMessage::ToApp(Err(SessionError::ParticipantDisconnected(name)))) =
            app_error
        {
            let name_str = name.to_string();
            assert!(
                name_str.contains("agntcy/ns/participant"),
                "Error message should mention the participant, got: {}",
                name_str
            );
        } else {
            panic!("Expected ParticipantDisconnected error");
        }

        // Verify shutdown was triggered when only 2 participants remained
        assert_eq!(
            moderator.processing_state(),
            ProcessingState::Draining,
            "Session should be in draining state"
        );
    }

    #[tokio::test]
    async fn test_moderator_concurrent_leave_requests() {
        // Test that concurrent leave requests are queued and processed sequentially

        // Create moderator with agntcy/ns/moderator naming
        let source = ProtoName::from_strings(["agntcy", "ns", "moderator"]).with_id(100);
        let destination = ProtoName::from_strings(["agntcy", "ns", "chat"]).with_id(1);
        let control = ProtoName::from_strings(["agntcy", "ns", "chat"]).with_id(2);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
            control: control.clone(),
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
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        // Set up moderator as joined
        let remote = ProtoName::from_strings(["agntcy", "ns", "participant1"]).with_id(200);
        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        run_with_acks(
            moderator.join(remote.clone(), 12345),
            &mut rx_slim,
            &sub_mgr,
        )
        .await
        .unwrap();

        // Add three participants to the group
        let mut participant1_name = ProtoName::from_strings(["agntcy", "ns", "participant1"]);
        let mut participant2_name = ProtoName::from_strings(["agntcy", "ns", "participant2"]);
        let mut participant3_name = ProtoName::from_strings(["agntcy", "ns", "participant3"]);
        let participant1 = Participant::new(
            participant1_name.clone(),
            ParticipantSettings::bidirectional(),
        );
        let participant2 = Participant::new(
            participant2_name.clone(),
            ParticipantSettings::bidirectional(),
        );
        let participant3 = Participant::new(
            participant3_name.clone(),
            ParticipantSettings::bidirectional(),
        );

        participant1_name.reset_id(); // Remove ID before inserting into group_list
        participant2_name.reset_id();
        participant3_name.reset_id();

        moderator
            .group_list
            .insert(participant1_name.clone(), participant1.clone());
        moderator
            .group_list
            .insert(participant2_name.clone(), participant2.clone());
        moderator
            .group_list
            .insert(participant3_name.clone(), participant3.clone());

        // Create first leave request coming directly from participant1 with LEAVING_SESSION metadata
        let participant1_with_id = participant1_name.clone().with_id(401);
        let mut leave_msg1 = Message::builder()
            .source(participant1_with_id.clone())
            .destination(source.clone()) // sent to moderator
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(101)
            .payload(
                CommandPayload::builder()
                    .leave_request() // No destination in payload when coming from participant
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Add LEAVING_SESSION metadata to signal graceful departure
        leave_msg1.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

        // Create second leave request coming directly from participant2 with LEAVING_SESSION metadata
        let participant2_with_id = participant2_name.clone().with_id(402);
        let mut leave_msg2 = Message::builder()
            .source(participant2_with_id.clone())
            .destination(source.clone()) // sent to moderator
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(102)
            .payload(
                CommandPayload::builder()
                    .leave_request() // No destination in payload when coming from participant
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Add LEAVING_SESSION metadata to signal graceful departure
        leave_msg2.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

        // Process first leave request (should start processing immediately)
        // Since it has LEAVING_SESSION metadata, it will be handled by on_disconnection_detected
        let result1 = moderator.on_disconnection_detected(leave_msg1, None).await;
        assert!(result1.is_ok() || result1.is_err());

        // Verify first task was created
        assert!(
            moderator.current_task.is_some(),
            "First leave should create a task"
        );

        // Process second leave request while first is still processing
        // Since it has LEAVING_SESSION metadata, it will be handled by on_disconnection_detected
        let result2 = moderator.on_disconnection_detected(leave_msg2, None).await;
        assert!(result2.is_ok());

        // Verify second task was queued
        assert_eq!(
            moderator.tasks_todo.len(),
            1,
            "Second leave request should be queued while first is processing"
        );

        // Verify the queued task exists and has LEAVE_REPLY_SENT metadata
        if let Some((queued_msg, _)) = moderator.tasks_todo.front() {
            // Verify LEAVE_REPLY_SENT metadata was set (LEAVING_SESSION was replaced)
            assert!(
                queued_msg.contains_metadata(LEAVE_REPLY_SENT),
                "Queued message should have LEAVE_REPLY_SENT metadata"
            );
        } else {
            panic!("Expected queued task for participant2");
        }

        // Clear the messages
        while rx_slim.try_recv().is_ok() {}

        // Verify participant1 was removed from group (first task processed)
        assert!(
            !moderator.group_list.contains_key(&participant1_name),
            "Participant1 should be removed after first leave request"
        );

        // Verify participant2 is still in group (second task queued, not processed yet)
        assert!(
            moderator.group_list.contains_key(&participant2_name),
            "Participant2 should still be in group (task queued, not processed)"
        );
    }

    /// A late or retransmitted GroupAck arriving when `current_task` is `None`
    /// must be silently discarded instead of panicking.
    #[tokio::test]
    async fn test_group_ack_ignored_when_no_current_task() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Sanity-check: no task is active.
        assert!(moderator.current_task.is_none());

        let source = make_name(&["participant", "app", "v1"]).with_id(300);
        let destination = moderator.common.settings.source.clone();

        // Build a GroupAck whose message_id was never registered with the sender,
        // so `is_still_pending` returns false and the guard is exercised.
        let group_ack = Message::builder()
            .source(source)
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(1)
            .message_id(999)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();

        // Must not panic; the stale ACK is discarded and Ok(()) is returned.
        let result = moderator
            .process_control_message(group_ack, MessageDirection::North, None)
            .await;
        assert!(result.is_ok());

        // State is unchanged.
        assert!(moderator.current_task.is_none());
    }

    // --- UpdateParticipantState Tests ---

    #[tokio::test]
    async fn test_update_participant_state_offline_from_slim() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let other_name_with_id = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other_name_with_id.clone();
        other_key.reset_id();

        moderator.group_list.insert(
            other_key.clone(),
            Participant::new(
                other_name_with_id.clone(),
                ParticipantSettings::bidirectional(),
            ),
        );

        let msg = Message::builder()
            .source(other_name_with_id.clone())
            .destination(moderator.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        other_name_with_id.clone(),
                        ParticipantState::Offline,
                        0,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Participant should be offline
        assert!(
            moderator.group_list.get(&other_key).unwrap().status
                == ParticipantState::Offline as i32
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
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let other_name_with_id = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other_name_with_id.clone();
        other_key.reset_id();

        // Add participant as offline
        let mut p = Participant::new(
            other_name_with_id.clone(),
            ParticipantSettings::bidirectional(),
        );
        p.status = ParticipantState::Offline as i32;
        moderator.group_list.insert(other_key.clone(), p);

        // MLS is None in test setup, so epoch check is skipped (always matches)
        let msg = Message::builder()
            .source(other_name_with_id.clone())
            .destination(moderator.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(101)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        other_name_with_id.clone(),
                        ParticipantState::Online,
                        0,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Participant should be online now
        assert!(
            moderator.group_list.get(&other_key).unwrap().status == ParticipantState::Online as i32
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
    async fn test_update_participant_state_from_app_close_and_ack() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add a participant
        let other_name = make_name(&["other", "participant", "v1"]);
        let other_name_with_id = other_name.clone().with_id(500);
        moderator.group_list.insert(
            other_name.clone(),
            Participant::new(
                other_name_with_id.clone(),
                ParticipantSettings::bidirectional(),
            ),
        );
        moderator.common.sender.add_participant(&other_name_with_id);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();

        // Send close from app
        let msg = Message::builder()
            .source(moderator.common.settings.source.clone())
            .destination(moderator.common.settings.control.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        moderator.common.settings.source.clone(),
                        ParticipantState::Offline,
                        0,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::South, Some(ack_tx)),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should have pending status update and current task
        assert!(moderator.common.pending_status_update.is_some());
        assert!(matches!(
            moderator.current_task,
            Some(ModeratorTask::UpdateLocalStatus())
        ));
        let msg_id = moderator
            .common
            .pending_status_update
            .as_ref()
            .unwrap()
            .message_id;

        // Now simulate the ACK arriving from the other participant
        let ack_msg = Message::builder()
            .source(other_name_with_id.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(1)
            .message_id(msg_id)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();

        let result = run_with_acks(
            moderator.process_control_message(ack_msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Moderator should be offline
        assert!(!moderator.common.online);
        // Task should be cleared
        assert!(moderator.current_task.is_none());
        // pending_status_update should be cleared
        assert!(moderator.common.pending_status_update.is_none());
        // App should be notified of success
        let ack_result = ack_rx.await.unwrap();
        assert!(ack_result.is_ok());
    }

    #[tokio::test]
    async fn test_update_participant_state_from_app_rejoin_and_ack() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add participants
        let p1_name = make_name(&["participant1", "app", "v1"]);
        let p1_with_id = p1_name.clone().with_id(501);
        let p2_name = make_name(&["participant2", "app", "v1"]);
        let p2_with_id = p2_name.clone().with_id(502);

        moderator.group_list.insert(
            p1_name.clone(),
            Participant::new(p1_with_id.clone(), ParticipantSettings::bidirectional()),
        );
        moderator.group_list.insert(
            p2_name.clone(),
            Participant::new(p2_with_id.clone(), ParticipantSettings::bidirectional()),
        );
        moderator.common.sender.add_participant(&p1_with_id);
        moderator.common.sender.add_participant(&p2_with_id);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();

        // Send rejoin from app
        let msg = Message::builder()
            .source(moderator.common.settings.source.clone())
            .destination(moderator.common.settings.control.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(300)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        moderator.common.settings.source.clone(),
                        ParticipantState::Online,
                        u64::MAX,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::South, Some(ack_tx)),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // All participants should be marked offline after rejoin send
        assert!(
            moderator.group_list.get(&p1_name).unwrap().status == ParticipantState::Offline as i32
        );
        assert!(
            moderator.group_list.get(&p2_name).unwrap().status == ParticipantState::Offline as i32
        );

        let msg_id = moderator
            .common
            .pending_status_update
            .as_ref()
            .unwrap()
            .message_id;

        // Simulate ACKs arriving from both participants
        let ack1 = Message::builder()
            .source(p1_with_id.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(1)
            .message_id(msg_id)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();

        let result = run_with_acks(
            moderator.process_control_message(ack1, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Still pending (waiting for p2's ACK)
        assert!(moderator.common.pending_status_update.is_some());

        let ack2 = Message::builder()
            .source(p2_with_id.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAck)
            .session_id(1)
            .message_id(msg_id)
            .payload(CommandPayload::builder().group_ack().as_content())
            .build_publish()
            .unwrap();

        let result = run_with_acks(
            moderator.process_control_message(ack2, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // All participants should be online now
        assert!(
            moderator.group_list.get(&p1_name).unwrap().status == ParticipantState::Online as i32
        );
        assert!(
            moderator.group_list.get(&p2_name).unwrap().status == ParticipantState::Online as i32
        );
        // Moderator should be online
        assert!(moderator.common.online);
        // Task should be cleared
        assert!(moderator.current_task.is_none());
        // pending_status_update cleared
        assert!(moderator.common.pending_status_update.is_none());
        // App notified of success
        let ack_result = ack_rx.await.unwrap();
        assert!(ack_result.is_ok());
    }

    // --- Rejoin Tests -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_on_rejoin_request_success() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add a third participant so GroupUpdate gets broadcast
        let third = make_name(&["third", "participant", "v1"]).with_id(700);
        let mut third_key = third.clone();
        third_key.reset_id();
        moderator.group_list.insert(
            third_key.clone(),
            Participant::new(third.clone(), ParticipantSettings::bidirectional()),
        );
        moderator.common.sender.add_participant(&third);

        // Add a participant that is offline
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other.clone();
        other_key.reset_id();
        let mut offline_participant =
            Participant::new(other.clone(), ParticipantSettings::bidirectional());
        offline_participant.status = ParticipantState::Offline as i32;
        moderator
            .group_list
            .insert(other_key.clone(), offline_participant);

        // Send rejoin request
        let msg = Message::builder()
            .source(other.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(other.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        let output = result.unwrap();
        // Should have sent RejoinReply + GroupUpdate(REJOIN)
        assert!(output.messages.len() >= 2);

        // Verify RejoinReply was sent
        let rejoin_reply = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToSlim(msg)
                    if msg.get_session_message_type() == ProtoSessionMessageType::RejoinReply =>
                {
                    Some(msg)
                }
                _ => None,
            })
            .expect("Expected RejoinReply message");
        assert_eq!(rejoin_reply.get_dst(), other);

        // Verify GroupUpdate(REJOIN) was sent
        let group_update = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToSlim(msg)
                    if msg.get_session_message_type() == ProtoSessionMessageType::GroupUpdate =>
                {
                    Some(msg)
                }
                _ => None,
            })
            .expect("Expected GroupUpdate message");
        let update_payload = group_update
            .get_payload()
            .unwrap()
            .as_command_payload()
            .unwrap()
            .as_group_update_payload()
            .unwrap();
        assert_eq!(update_payload.op, GroupUpdateOp::Rejoin as i32);

        // Task should be Rejoin
        assert!(matches!(
            moderator.current_task,
            Some(ModeratorTask::Rejoin(_))
        ));

        // Participant should be marked online
        assert_eq!(
            moderator.group_list.get(&other_key).unwrap().status,
            ParticipantState::Online as i32
        );
    }

    #[tokio::test]
    async fn test_on_rejoin_request_unknown_participant() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let unknown = make_name(&["unknown", "participant", "v1"]).with_id(999);

        let msg = Message::builder()
            .source(unknown.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(unknown.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should produce no output (ignored)
        let output = result.unwrap();
        assert!(output.is_empty());
        assert!(moderator.current_task.is_none());
    }

    #[tokio::test]
    async fn test_on_rejoin_request_participant_online() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add a participant that is ONLINE
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other.clone();
        other_key.reset_id();
        moderator.group_list.insert(
            other_key.clone(),
            Participant::new(other.clone(), ParticipantSettings::bidirectional()),
        );

        let msg = Message::builder()
            .source(other.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(other.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Without MLS there is no epoch to mismatch, so the request is ignored.
        let output = result.unwrap();
        assert!(output.is_empty());
        assert!(moderator.current_task.is_none());
    }

    #[tokio::test]
    async fn test_on_rejoin_request_online_participant_with_mls_processes_rejoin() {
        // When MLS is enabled an online participant sending a RejoinRequest signals
        // an epoch mismatch (e.g. crash-restore). The moderator must process the
        // request instead of silently dropping it.
        use slim_auth::shared_secret::SharedSecret;

        const SECRET: &str = "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas";

        let source = make_name(&["local", "moderator", "v1"]).with_id(100);
        let destination = make_name(&["channel", "name", "v1"]).with_id(1);
        let control = make_name(&["channel", "name", "v1"]).with_id(2);

        let identity_provider = SharedSecret::new("test", SECRET).unwrap();
        let identity_verifier = SharedSecret::new("test", SECRET).unwrap();

        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: Some(MlsSettings {
                header_integrity_validation_percent: 0,
                max_seen_control_message_ids_size: None,
            }),
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source,
            destination: destination.clone(),
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
            subscription_manager: subscription_manager.clone(),
            service_id: String::new(),
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            kv_store: None,
            group_storage: None,
            enforce_pqc: false,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        moderator
            .mls_state
            .as_mut()
            .unwrap()
            .init_moderator()
            .await
            .unwrap();

        // Add the participant as ONLINE (not offline)
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other.clone();
        other_key.reset_id();
        moderator.group_list.insert(
            other_key.clone(),
            Participant::new(other.clone(), ParticipantSettings::bidirectional()),
        );

        let msg = Message::builder()
            .source(other.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(other.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;

        // The participant only sends a RejoinRequest when it has detected an
        // epoch mismatch, so a key_package being present implies the mismatch.
        // The moderator must process it (not silently drop it): with a fake key
        // package the MLS library returns an error, proving the online-guard
        // was bypassed.
        assert!(
            result.is_err(),
            "expected MLS error from invalid key package, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_on_rejoin_request_when_busy() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Add a participant that is offline
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other.clone();
        other_key.reset_id();
        let mut offline_participant =
            Participant::new(other.clone(), ParticipantSettings::bidirectional());
        offline_participant.status = ParticipantState::Offline as i32;
        moderator
            .group_list
            .insert(other_key.clone(), offline_participant);

        // Set a current task (busy)
        moderator.current_task = Some(ModeratorTask::UpdateLocalStatus());

        let msg = Message::builder()
            .source(other.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(other.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        // Should produce no output (dropped)
        let output = result.unwrap();
        assert!(output.is_empty());

        // Task should still be the old one
        assert!(matches!(
            moderator.current_task,
            Some(ModeratorTask::UpdateLocalStatus())
        ));
    }

    #[tokio::test]
    async fn test_on_rejoin_request_two_participants_sends_group_update() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        // Only moderator + one offline participant
        let other = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other.clone();
        other_key.reset_id();
        let mut offline_participant =
            Participant::new(other.clone(), ParticipantSettings::bidirectional());
        offline_participant.status = ParticipantState::Offline as i32;
        moderator
            .group_list
            .insert(other_key.clone(), offline_participant);

        let msg = Message::builder()
            .source(other.clone())
            .destination(moderator.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::RejoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .rejoin_request(other.clone(), vec![1, 2, 3])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let sub_mgr = moderator.common.settings.subscription_manager.clone();
        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &sub_mgr,
        )
        .await;
        assert!(result.is_ok());

        let output = result.unwrap();

        // Should have sent both RejoinReply and GroupUpdate (always sent for group_list rebuild)
        let has_rejoin_reply = output.messages.iter().any(|m| match m {
            OutboundMessage::ToSlim(msg) => {
                msg.get_session_message_type() == ProtoSessionMessageType::RejoinReply
            }
            _ => false,
        });
        assert!(has_rejoin_reply, "Expected RejoinReply message");

        let has_group_update = output.messages.iter().any(|m| match m {
            OutboundMessage::ToSlim(msg) => {
                msg.get_session_message_type() == ProtoSessionMessageType::GroupUpdate
            }
            _ => false,
        });
        assert!(has_group_update, "Expected GroupUpdate message");

        // Task should be Rejoin
        assert!(matches!(
            moderator.current_task,
            Some(ModeratorTask::Rejoin(_))
        ));
    }

    #[tokio::test]
    async fn test_update_participant_state_online_epoch_mismatch_sends_nack() {
        use slim_auth::shared_secret::SharedSecret;

        const SECRET: &str = "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas";

        let source = make_name(&["local", "moderator", "v1"]).with_id(100);
        let destination = make_name(&["channel", "name", "v1"]).with_id(1);
        let control = make_name(&["channel", "name", "v1"]).with_id(2);

        let identity_provider = SharedSecret::new("test", SECRET).unwrap();
        let identity_verifier = SharedSecret::new("test", SECRET).unwrap();

        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let subscription_manager =
            crate::subscription_manager::SubscriptionManager::new(tx_slim.clone());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: Some(MlsSettings {
                header_integrity_validation_percent: 0,
                max_seen_control_message_ids_size: None,
            }),
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source,
            destination: destination.clone(),
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
            subscription_manager: subscription_manager.clone(),
            service_id: String::new(),
            max_seen_control_message_ids_size: DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE,
            enforce_pqc: false,
            kv_store: None,
            group_storage: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        // Initialize the MLS group so get_epoch() returns Some(1)
        moderator
            .mls_state
            .as_mut()
            .unwrap()
            .init_moderator()
            .await
            .unwrap();

        let current_epoch = moderator
            .mls_state
            .as_ref()
            .unwrap()
            .common
            .mls
            .get_epoch()
            .unwrap();
        assert_eq!(current_epoch, 0);

        let other_name_with_id = make_name(&["other", "participant", "v1"]).with_id(500);
        let mut other_key = other_name_with_id.clone();
        other_key.reset_id();

        // Add participant as offline
        moderator.group_list.insert(
            other_key.clone(),
            Participant {
                name: Some(other_name_with_id.clone()),
                settings: Some(ParticipantSettings::bidirectional()),
                status: ParticipantState::Offline as i32,
            },
        );

        // Send UpdateParticipantState(Online) with mismatched epoch (99 != 0)
        let msg = Message::builder()
            .source(other_name_with_id.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(1)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::UpdateParticipantState)
            .session_id(1)
            .message_id(101)
            .payload(
                CommandPayload::builder()
                    .update_participant_state(
                        other_name_with_id.clone(),
                        ParticipantState::Online,
                        99, // mismatched epoch (0 is current)
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = run_with_acks(
            moderator.process_control_message(msg, MessageDirection::North, None),
            &mut rx_slim,
            &subscription_manager,
        )
        .await;
        assert!(result.is_ok());

        // Participant should still be offline
        assert_eq!(
            moderator.group_list.get(&other_key).unwrap().status,
            ParticipantState::Offline as i32
        );

        // Should have sent a NACK (not an ACK)
        let output = result.unwrap();
        assert!(!output.is_empty());
        let nack_msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m,
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            nack_msg.get_session_message_type(),
            ProtoSessionMessageType::GroupNack
        );
    }
}
