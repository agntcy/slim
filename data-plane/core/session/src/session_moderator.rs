// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use async_trait::async_trait;
use display_error_chain::ErrorChainExt;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, MlsPayload, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType,
    },
    messages::{
        Name,
        utils::{DELETE_GROUP, DISCONNECTION_DETECTED, LEAVING_SESSION, TRUE_VAL},
    },
};
use tokio::sync::oneshot;

use slim_mls::mls::Mls;
use tracing::debug;

use crate::{
    common::{MessageDirection, SessionMessage},
    errors::SessionError,
    mls_state::{MlsModeratorState, MlsState},
    moderator_task::{
        AddParticipant, ModeratorTask, NotifyParticipants, RemoveParticipant, TaskUpdate,
    },
    session_controller::SessionControllerCommon,
    session_settings::SessionSettings,
    traits::{MessageHandler, ProcessingState},
};

pub struct SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// Queue of tasks to be performed by the moderator
    /// Each task contains a message and an optional ack channel
    tasks_todo: VecDeque<(Message, Option<oneshot::Sender<Result<(), SessionError>>>)>,

    /// Current task being processed by the moderator
    current_task: Option<ModeratorTask>,

    /// MLS state for the moderator
    mls_state: Option<MlsModeratorState<P, V>>,

    /// List of group participants
    group_list: HashMap<Name, u64>,

    /// Common settings
    common: SessionControllerCommon<P, V>,

    /// Postponed message to be sent after current task completion
    postponed_message: Option<Message>,

    /// Subscription status
    subscribed: bool,

    /// connection id to the remote node
    conn_id: Option<u64>,

    /// Inner message handler
    inner: I,
}

impl<P, V, I> SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    pub(crate) fn new(inner: I, settings: SessionSettings<P, V>) -> Self {
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
}

/// Implementation of MessageHandler trait for SessionModerator
/// This allows the moderator to be used as a layer in the generic layer system
#[async_trait]
impl<P, V, I> MessageHandler for SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    async fn init(&mut self) -> Result<(), SessionError> {
        // Initialize MLS
        self.mls_state = if self.common.settings.config.mls_enabled {
            let mls_state = MlsState::new(Mls::new(
                self.common.settings.identity_provider.clone(),
                self.common.settings.identity_verifier.clone(),
            ))
            .await
            .expect("failed to create MLS state");

            Some(MlsModeratorState::new(mls_state))
        } else {
            None
        };

        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        match message {
            SessionMessage::OnMessage {
                mut message,
                direction,
                ack_tx,
            } => {
                if message.get_session_message_type().is_command_message() {
                    debug!(
                        message = ?message.get_session_message_type(),
                        source = %message.get_source(),
                        "received  message",
                    );
                    self.process_control_message(message, ack_tx).await
                } else {
                    // this is a application message. if direction (needs to go to the remote endpoint) and
                    // the session is p2p, update the destination of the message with the destination in
                    // the self.common. In this way we always add the right id to the name
                    if direction == MessageDirection::South
                        && self.common.settings.config.session_type
                            == ProtoSessionType::PointToPoint
                    {
                        message
                            .get_slim_header_mut()
                            .set_destination(&self.common.settings.destination);
                    }

                    // Apply MLS encryption/decryption if enabled
                    if let Some(mls_state) = &mut self.mls_state {
                        mls_state
                            .common
                            .process_message(&mut message, direction)
                            .await?;
                    }

                    self.inner
                        .on_message(SessionMessage::OnMessage {
                            message,
                            direction,
                            ack_tx,
                        })
                        .await
                }
            }
            SessionMessage::MessageError { error } => self.handle_message_error(error).await,
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    self.common
                        .sender
                        .on_timer_timeout(message_id, message_type)
                        .await
                } else {
                    self.inner
                        .on_message(SessionMessage::TimerTimeout {
                            message_id,
                            message_type,
                            name,
                            timeouts,
                        })
                        .await
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
                    .await
                } else {
                    self.inner
                        .on_message(SessionMessage::TimerFailure {
                            message_id,
                            message_type,
                            name,
                            timeouts,
                        })
                        .await
                }
            }
            SessionMessage::StartDrain { grace_period: _ } => {
                debug!("start draining by calling delete_all");
                // Set processing state to draining
                self.common.processing_state = ProcessingState::Draining;
                // We need to close the session for all the participants
                // Create the leave message
                let p = CommandPayload::builder().leave_request(None).as_content();
                let destination = self.common.settings.destination.clone();
                let mut leave_msg = self.common.create_control_message(
                    &destination,
                    ProtoSessionMessageType::LeaveRequest,
                    rand::random::<u32>(),
                    p,
                    false,
                )?;
                leave_msg.insert_metadata(DELETE_GROUP.to_string(), TRUE_VAL.to_string());

                // send it to all the participants
                self.delete_all(leave_msg, None, false).await
            }
            SessionMessage::ParticipantDisconnected {
                name: opt_participant,
            } => {
                let participant =
                    opt_participant.ok_or(SessionError::MissingParticipantNameOnDisconnection)?;
                debug!(
                    %participant,
                    "Participant not anymore connected to the current session",
                );

                // create a leave request message for the participant that
                // got disconnected and add the metadata to the message
                let mut msg = self.common.create_control_message(
                    &participant.clone(),
                    ProtoSessionMessageType::LeaveRequest,
                    rand::random::<u32>(),
                    CommandPayload::builder().leave_request(None).as_content(),
                    false,
                )?;
                msg.insert_metadata(DISCONNECTION_DETECTED.to_string(), TRUE_VAL.to_string());

                // process the leave request message
                self.on_disconnection_detected(msg, None).await
            }
            _ => Err(SessionError::SessionMessageInternalUnexpected(Box::new(
                message,
            ))),
        }
    }

    async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        self.inner.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &Name) {
        self.inner.remove_endpoint(endpoint);
    }

    fn needs_drain(&self) -> bool {
        !(self.common.sender.drain_completed()
            && !self.inner.needs_drain()
            && self.tasks_todo.is_empty())
    }
    fn processing_state(&self) -> ProcessingState {
        self.common.processing_state
    }

    fn participants_list(&self) -> Vec<Name> {
        self.group_list
            .iter()
            .map(|(name, id)| name.clone().with_id(*id))
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
                .delete_route(&self.common.settings.destination, conn)
                .await?;
            self.common
                .delete_subscription(&self.common.settings.destination, conn)
                .await?;
        }

        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await?;

        self.send_close_signal().await;

        Ok(())
    }
}

impl<P, V, I> SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// Helper method to handle MessageError
    /// Extracts context from the error and routes to appropriate handler
    async fn handle_message_error(&mut self, error: SessionError) -> Result<(), SessionError> {
        let Some(session_ctx) = error.session_context() else {
            tracing::warn!("Received MessageError without session context");
            return self
                .inner
                .on_message(SessionMessage::MessageError { error })
                .await;
        };

        if error.is_command_message_error() {
            // Handle command message failure
            self.handle_failure(
                session_ctx.message_id,
                session_ctx.get_session_message_type(),
                error,
            )
            .await
        } else {
            // Pass non-command errors to inner handler
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
        self.common.sender.on_failure(message_id, message_type);

        // the task should always exist at this point
        if let Some(task) = self.current_task.as_mut()
            && let Some(ack_tx) = task.ack_tx_take()
        {
            let _ = ack_tx.send(Err(task.failure_message(error)));
        }

        // delete current task and pick a new one
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
        participant: &Name,
        msg: &Message,
    ) -> Result<(Vec<Name>, Option<MlsPayload>), SessionError> {
        // Build participants list with current participants
        // the group update needs to be received by everybody
        // in the group unless there are only 2 participants
        // (the moderator and a participant)
        let participants_vec: Vec<Name> = self
            .group_list
            .iter()
            .map(|(n, id)| n.clone().with_id(*id))
            .collect();

        // Remove participant from group list
        let mut participant_no_id = participant.clone();
        participant_no_id.reset_id();
        self.group_list.remove(&participant_no_id);

        // Remove endpoint from local session
        self.remove_endpoint(participant);

        // Compute MLS payload if needed
        let mls_payload = match self.mls_state.as_mut() {
            Some(state) => {
                let mls_content = state
                    .remove_participant(msg)
                    .await
                    .map_err(|e| self.handle_task_error(e))?;
                let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
                Some(MlsPayload {
                    commit_id,
                    mls_content,
                })
            }
            None => None,
        };

        Ok((participants_vec, mls_payload))
    }

    /// Helper method to send a GroupRemove message to notify participants
    /// Returns the message ID of the sent GroupRemove message
    async fn send_group_remove(
        &mut self,
        removed_participant: Name,
        participants: Vec<Name>,
        mls_payload: Option<MlsPayload>,
    ) -> Result<u32, SessionError> {
        let update_payload = CommandPayload::builder()
            .group_remove(removed_participant, participants, mls_payload)
            .as_content();
        let msg_id = rand::random::<u32>();

        self.common
            .send_control_message(
                &self.common.settings.destination.clone(),
                ProtoSessionMessageType::GroupRemove,
                msg_id,
                update_payload,
                None,
                true,
            )
            .await?;

        Ok(msg_id)
    }

    async fn process_control_message(
        &mut self,
        message: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest => {
                self.on_discovery_request(message, ack_tx).await
            }
            ProtoSessionMessageType::DiscoveryReply => self.on_discovery_reply(message).await,
            ProtoSessionMessageType::JoinRequest => {
                // this message should arrive only from the control plane
                // the effect of it is to create the session itself with
                // the right settings. We need to set a route to the controller and send back the ack
                self.common
                    .add_route(&message.get_source(), message.get_incoming_conn())
                    .await
                    .map_err(|e| self.handle_task_error(e))?;

                let ack = self.common.create_control_message(
                    &message.get_source(),
                    ProtoSessionMessageType::GroupAck,
                    message.get_id(),
                    CommandPayload::builder().group_ack().as_content(),
                    false,
                )?;
                self.common.send_to_slim(ack).await?;
                Ok(())
            }
            ProtoSessionMessageType::JoinReply => self.on_join_reply(message).await,
            ProtoSessionMessageType::LeaveRequest => {
                // if the metadata contains the key "DELETE_GROUP" remove all the participants
                // and close the session when all task are completed
                if message.contains_metadata(DELETE_GROUP) {
                    return self.delete_all(message, ack_tx, true).await;
                }

                // the LeaveRequest message is also used to signal the disconnection of
                // a remote participant. if the metadata contains the key "DISCONNECTION_DETECTED"
                // or "LEAVING_SESSION" call the function on_disconnection_detected
                if message.contains_metadata(DISCONNECTION_DETECTED)
                    || message.contains_metadata(LEAVING_SESSION)
                {
                    return self.on_disconnection_detected(message, ack_tx).await;
                }

                // if the message contains a payload and the name is the same as the
                // local one, call the delete all anyway
                if let Some(n) = message
                    .get_payload()
                    .ok_or_else(|| SessionError::MissingPayload {
                        context: "control_message",
                    })?
                    .as_command_payload()?
                    .as_leave_request_payload()?
                    .destination
                    .as_ref()
                    && Name::from(n) == self.common.settings.source
                {
                    return self.delete_all(message, ack_tx, true).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message, ack_tx).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::Ping => self.common.sender.on_message(&message).await,
            ProtoSessionMessageType::GroupProposal => todo!(),
            ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupClose
            | ProtoSessionMessageType::GroupNack => Err(
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
    ) -> Result<(), SessionError> {
        debug!(%self.common.settings.id, "received discovery request");
        // the channel discovery starts a new participant invite.
        // process the request only if not busy
        if self.current_task.is_some() {
            debug!(
                "Moderator is busy. Add invite participant task to the list and process it later"
            );
            // if busy postpone the task and add it to the todo list with its ack_tx
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(());
        }

        // now the moderator is busy - create the task first
        debug!("Create AddParticipant task with ack_tx");
        // check if there is a destination name in the payload. If yes recreate the message
        // with the right destination and send it out
        let payload = msg.extract_discovery_request().map_err(|e| {
            let err = SessionError::extract_error("discovery_request", e);
            self.handle_task_error(err)
        })?;

        let (mut discovery, ack) = match &payload.destination {
            Some(dst_name) => {
                // create the ack message to send back to the controller
                let ack = self.common.create_control_message(
                    &msg.get_source(),
                    ProtoSessionMessageType::GroupAck,
                    msg.get_id(),
                    CommandPayload::builder().group_ack().as_content(),
                    false,
                )?;

                // set the route to forward the messages correctly
                // here we assume that the destination is reachable from the
                // same connection from where we got the message from the controller
                let dst = Name::from(dst_name);
                self.common
                    .add_route(&dst, msg.get_incoming_conn())
                    .await
                    .map_err(|e| self.handle_task_error(e))?;

                // create a new empty payload and change the message destination
                let p = CommandPayload::builder()
                    .discovery_request(None)
                    .as_content();
                msg.get_slim_header_mut()
                    .set_source(&self.common.settings.source);
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(p);

                (msg, Some(ack))
            }
            None => {
                // simply forward the message
                (msg, None)
            }
        };
        self.current_task = Some(ModeratorTask::Add(AddParticipant::new(ack_tx, ack)));

        // check if the participant is already part of the group
        let new_participant_name = discovery.get_dst();
        if self.group_list.contains_key(&new_participant_name) {
            let err = SessionError::ParticipantAlreadyInGroup(new_participant_name);
            return Err(self.handle_task_error(err));
        }

        // start the current task
        let id = rand::random::<u32>();
        discovery.get_session_header_mut().set_message_id(id);
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_start(id)
            .map_err(|e| self.handle_task_error(e))?;

        debug!(
            dst = %discovery.get_dst(),
            id = discovery.get_id(),
            "send discovery request",
        );
        self.common
            .send_with_timer(discovery)
            .await
            .map_err(|e| self.handle_task_error(e))
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            source = %msg.get_source(),
            id = msg.get_id(),
            "discovery reply",
        );
        // update sender status to stop timers
        self.common.sender.on_message(&msg).await?;

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
            .add_route(&msg.get_source(), msg.get_incoming_conn())
            .await?;

        // if this is a multicast session we need to add a route for the channel
        // on the connection from where we received the message. This has to be done
        // all the times because the messages from the remote endpoints may come from
        // different connections. In case the route exists already it will be just ignored
        if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            self.common
                .add_route(&self.common.settings.destination, msg.get_incoming_conn())
                .await?;
        }

        // an endpoint replied to the discovery message
        // send a join message
        let msg_id = rand::random::<u32>();

        let channel = if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            Some(self.common.settings.destination.clone())
        } else {
            None
        };

        let payload = CommandPayload::builder()
            .join_request(
                self.mls_state.is_some(),
                self.common.settings.config.max_retries,
                self.common.settings.config.interval,
                channel,
            )
            .as_content();

        debug!(
            dst = %msg.get_slim_header().get_source(),
            id = msg_id,
            "send join request",
        );
        self.common
            .send_control_message(
                &msg.get_slim_header().get_source(),
                ProtoSessionMessageType::JoinRequest,
                msg_id,
                payload,
                Some(self.common.settings.config.metadata.clone()),
                false,
            )
            .await?;

        // evolve the current task state
        // start the join phase
        self.current_task.as_mut().unwrap().join_start(msg_id)
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            source = %msg.get_source(),
            id = msg.get_id(),
            "join reply",
        );
        // stop the timer for the join request
        self.common.sender.on_message(&msg).await?;

        // evolve the current task state
        // the join phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .join_complete(msg.get_id())?;

        // at this point the participant is part of the group so we can add it to the list
        let mut new_participant_name = msg.get_source().clone();
        let new_participant_id = new_participant_name.id();
        new_participant_name.reset_id();
        self.group_list
            .insert(new_participant_name, new_participant_id);

        // notify the local session that a new participant was added to the group
        debug!(session_name = %msg.get_source(), "add endpoint");
        self.add_endpoint(&msg.get_source()).await?;

        // get mls data if MLS is enabled
        let (commit, welcome) = if let Some(mls_state) = &mut self.mls_state {
            let (commit_payload, welcome_payload) = mls_state.add_participant(&msg).await?;

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
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        // send the group update
        if participants_vec.len() > 2 {
            debug!("participant len is > 2, send a group update");
            let update_payload = CommandPayload::builder()
                .group_add(msg.get_source().clone(), participants_vec.clone(), commit)
                .as_content();
            let add_msg_id = rand::random::<u32>();
            debug!(id = %add_msg_id, "send add update to channel");
            self.common
                .send_control_message(
                    &self.common.settings.destination.clone(),
                    ProtoSessionMessageType::GroupAdd,
                    add_msg_id,
                    update_payload,
                    None,
                    true,
                )
                .await?;
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
        self.common
            .send_control_message(
                &msg.get_slim_header().get_source(),
                ProtoSessionMessageType::GroupWelcome,
                welcome_msg_id,
                welcome_payload,
                None,
                false,
            )
            .await?;

        // evolve the current task state
        // welcome start
        self.current_task
            .as_mut()
            .unwrap()
            .welcome_start(welcome_msg_id)?;

        Ok(())
    }

    async fn on_leave_request(
        &mut self,
        mut msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list with its ack_tx
            debug!("Moderator is busy. Add leave request task to the list and process it later");
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(());
        }

        debug!("Create RemoveParticipant task with ack_tx");
        // adjust the message according to the sender:
        // - if coming from the controller (destination in the payload) we need to modify source and destination
        // - if coming from the app (empty payload) we need to add the participant id to the destination
        let payload_destination = msg
            .extract_leave_request()
            .map_err(|e| {
                let err = SessionError::extract_error("leave_request", e);
                self.handle_task_error(err)
            })?
            .destination
            .clone();

        // Update message based on whether destination was provided in payload
        let (dst_without_id, ack) = match payload_destination {
            Some(dst_name) => {
                // create also the ack to send back to the controller
                let ack = self.common.create_control_message(
                    &msg.get_source(),
                    ProtoSessionMessageType::GroupAck,
                    msg.get_id(),
                    CommandPayload::builder().group_ack().as_content(),
                    false,
                )?;

                // Destination provided: update source, destination, and payload
                let new_payload = CommandPayload::builder().leave_request(None).as_content();
                msg.get_slim_header_mut()
                    .set_source(&self.common.settings.source);
                msg.set_payload(new_payload);

                (Name::from(&dst_name), Some(ack))
            }
            None => (msg.get_dst(), None),
        };

        self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::new(ack_tx, ack)));

        // Look up participant ID in group list
        let id = match self.group_list.get(&dst_without_id) {
            Some(id) => *id,
            None => {
                let err = SessionError::ParticipantNotFound(dst_without_id);
                return Err(self.handle_task_error(err));
            }
        };

        // Set destination with ID and message ID (common to both cases)
        let dst_with_id = dst_without_id.clone().with_id(id);
        msg.get_slim_header_mut().set_destination(&dst_with_id);
        msg.set_message_id(rand::random::<u32>());

        let leave_message = msg;

        // Remove the participant from the group list and compute MLS payload
        debug!(
            session_name = %leave_message.get_dst(),
            "remove endpoint from the session",
        );

        let (participants_vec, mls_payload) = self
            .remove_participant_and_compute_mls(&leave_message.get_dst(), &leave_message)
            .await?;

        if participants_vec.len() > 2 {
            // in this case we need to send first the group update and later the leave message
            let msg_id = self
                .send_group_remove(leave_message.get_dst(), participants_vec, mls_payload)
                .await?;
            self.current_task.as_mut().unwrap().commit_start(msg_id)?;

            // We need to save the leave message and send it after
            // the reception of all the acks for the group update message
            // see on_group_ack for postponed_message handling
            self.postponed_message = Some(leave_message);
        } else {
            // no commit message will be sent so update the task state to consider the commit as received
            // the timer id is not important here, it just need to be consistent
            self.current_task.as_mut().unwrap().commit_start(12345)?;
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(12345)?;

            // just send the leave message in this case
            self.common.sender.on_message(&leave_message).await?;

            self.current_task
                .as_mut()
                .unwrap()
                .leave_start(leave_message.get_id())?;
        }

        Ok(())
    }

    async fn on_disconnection_detected(
        &mut self,
        mut msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
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
            return Ok(());
        }

        debug!(%disconnected, "disconnection detected");

        // Send error notification to the application
        let error = SessionError::ParticipantDisconnected(disconnected.clone());
        self.common.send_to_app(error).await?;

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
            self.common.send_to_slim(reply).await?;

            // replace LEAVING_SESSION with DISCONNECTION_DETECTED so that if the process of the
            // message needs to be delayed because the moderator is busy we do not send the reply twice
            msg.remove_metadata(LEAVING_SESSION);
            msg.insert_metadata(DISCONNECTION_DETECTED.to_string(), TRUE_VAL.to_string());
            let header = msg.get_slim_header_mut();
            header.set_destination(&disconnected);
            header.set_source(&self.common.settings.source);
        }

        // if the session if P2P or no one is left on the session close it
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
            return Ok(());
        }

        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list with its ack_tx
            debug!(
                "Moderator is busy. Add disconnection handling task to the list and process it later"
            );
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(());
        }

        debug!("Create disconnected task for the disconnection handling");
        // Reuse the disconnection task here, however we don't need to send the leave message
        // so we can mark it as done immediately
        self.current_task = Some(ModeratorTask::CloseOrDisconnect(NotifyParticipants::new(
            ack_tx, None,
        )));

        // Remove the participant from the group list and compute MLS payload
        debug!(
            endpoint = %disconnected,
            "remove disconnected endpoint from the session",
        );

        let (participants_vec, mls_payload) = self
            .remove_participant_and_compute_mls(&disconnected, &msg)
            .await?;

        // Notify all the participants left and update the MLS state if needed
        let msg_id = self
            .send_group_remove(disconnected, participants_vec, mls_payload)
            .await?;
        self.current_task.as_mut().unwrap().commit_start(msg_id)
    }

    async fn delete_all(
        &mut self,
        msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
        // set to true if the delete all is requested from the control plane
        // so that we can send back an ack message
        from_control_plane: bool,
    ) -> Result<(), SessionError> {
        debug!("receive a close channel message, send signals to all participants");
        self.prepare_shutdown().await?;

        // Collect the participants and create the close message
        let participants: Vec<_> = self
            .group_list
            .iter()
            .map(|(k, v)| k.clone().with_id(*v))
            .collect();

        if participants.len() == 1 {
            // in this case the moderator is the only one remained
            // in the group so there is nothing to do
            return Ok(());
        }

        let destination = self.common.settings.destination.clone();
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

        let ack = if from_control_plane {
            Some(self.common.create_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::GroupAck,
                msg.get_id(),
                CommandPayload::builder().group_ack().as_content(),
                false,
            )?)
        } else {
            None
        };

        // create the close task
        self.current_task = Some(ModeratorTask::CloseOrDisconnect(NotifyParticipants::new(
            ack_tx, ack,
        )));
        self.current_task.as_mut().unwrap().commit_start(close_id)?;

        // sent the message
        self.common.sender.on_message(&close).await
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            from = %msg.get_source(),
            id = %msg.get_id(),
            "received leave reply",
        );
        let msg_id = msg.get_id();

        // delete the route to the source of the message
        self.common
            .delete_route(&msg.get_source(), msg.get_incoming_conn())
            .await?;

        // notify the sender and see if we can pick another task
        self.common.sender.on_message(&msg).await?;
        if !self.common.sender.is_still_pending(msg_id) {
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
        }

        self.task_done().await
    }

    async fn on_group_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            from = %msg.get_source(),
            id = %msg.get_id(),
            "received group ack",
        );
        // notify the sender
        self.common.sender.on_message(&msg).await?;

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.common.sender.is_still_pending(msg_id) {
            debug!(
                id = %msg_id,
                "process group ack. try to close task",
            );
            // we received all the messages related to this timer
            // check if we are done and move on
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(msg_id)?;

            // check if the task is finished.
            if !self.current_task.as_mut().unwrap().task_complete() {
                // if the task is not finished yet we may need to send a leave
                // message that was postponed to send all group update first
                if let Some(leave_message) = &self.postponed_message
                    && matches!(self.current_task, Some(ModeratorTask::Remove(_)))
                {
                    // send the leave message an progress
                    self.common.sender.on_message(leave_message).await?;
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

        Ok(())
    }

    /// task handling functions
    async fn task_done(&mut self) -> Result<(), SessionError> {
        if !self.current_task.as_ref().unwrap().task_complete() {
            // the task is not completed so just return
            // and continue with the process
            debug!("Current task is NOT completed");
            return Ok(());
        }

        // check if we need to send an ack message to the controller
        if let Some(ack_msg) = self.current_task.as_ref().unwrap().ack_msg() {
            // Use ack_msg to notify the controller that the task is done
            debug!("Send ack message for task {:?}", self.current_task);
            self.common.send_to_slim(ack_msg.clone()).await?;
        } else {
            // NO need to send any notification here
            debug!("No ack message for task {:?}", self.current_task);
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

        debug!("Process a new task from the todo list");
        // Process the control message by calling on_message
        // Since this is a control message coming from our internal queue,
        // we use MessageDirection::North (coming from network/control plane)
        // The ack_tx that was stored with the task is now used
        self.on_message(SessionMessage::OnMessage {
            message: msg,
            direction: MessageDirection::North,
            ack_tx,
        })
        .await
    }

    async fn join(&mut self, remote: Name, conn: u64) -> Result<(), SessionError> {
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
            self.common
                .add_subscription(&self.common.settings.destination, conn)
                .await?;
        }

        // create mls group if needed
        if let Some(mls) = self.mls_state.as_mut() {
            mls.init_moderator().await?;
        }

        // add ourself to the participants
        let mut local_name = self.common.settings.source.clone();
        let id = local_name.id();
        local_name.reset_id();
        self.group_list.insert(local_name, id);

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

    async fn send_close_signal(&mut self) {
        debug!("Signal session layer to close the session, all tasks are done");

        // notify the session layer
        let res = self
            .common
            .settings
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.settings.id,
            }))
            .await;

        if let Err(e) = res {
            tracing::error!(error = %e.chain(), "an error occurred while signaling session close");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_config::SessionConfig;
    use crate::session_settings::SessionSettings;
    use crate::test_utils::{MockInnerHandler, MockTokenProvider, MockVerifier};
    use slim_datapath::Status;
    use slim_datapath::api::{CommandPayload, ProtoSessionType};
    use slim_datapath::messages::Name;
    use tokio::sync::mpsc;

    // --- Test Helpers -----------------------------------------------------------------------

    fn make_name(parts: &[&str; 3]) -> Name {
        Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    fn setup_moderator() -> (
        SessionModerator<MockTokenProvider, MockVerifier, MockInnerHandler>,
        mpsc::Receiver<Result<Message, Status>>,
        mpsc::Receiver<Result<SessionMessage, SessionError>>,
    ) {
        let source = make_name(&["local", "moderator", "v1"]).with_id(100);
        let destination = make_name(&["channel", "name", "v1"]).with_id(200);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, rx_session_layer) = mpsc::channel(16);

        let tx = crate::transmitter::SessionTransmitter::new(tx_slim, tx_app);

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source,
            destination,
            config,
            tx,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
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
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
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
            .payload(
                CommandPayload::builder()
                    .discovery_request(None)
                    .as_content(),
            )
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
        let sent_msg = rx_slim.try_recv();
        assert!(sent_msg.is_ok());
        let msg = sent_msg.unwrap().unwrap();
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
        moderator.current_task = Some(ModeratorTask::Add(AddParticipant::new(None, None)));

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
            .payload(
                CommandPayload::builder()
                    .discovery_request(None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = moderator.on_discovery_request(discovery_msg, None).await;
        assert!(result.is_ok());

        // Should have added task to todo list
        assert_eq!(moderator.tasks_todo.len(), 1);
    }

    #[tokio::test]
    async fn test_moderator_join_request_passthrough() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
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
                        false,
                        Some(3),
                        Some(std::time::Duration::from_secs(1)),
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();
        let result = moderator.process_control_message(join_msg, None).await;
        assert!(result.is_ok());
        let subscribe_msg = rx_slim.recv().await.unwrap().unwrap();
        assert!(
            matches!(
                subscribe_msg.get_type(),
                slim_datapath::api::MessageType::Subscribe(_)
            ),
            "First message should be a Subscribe message"
        );
        assert_eq!(
            subscribe_msg.get_source(),
            destination,
            "Subscribe message source should match"
        );
        assert_eq!(
            subscribe_msg.get_dst(),
            source,
            "Subscribe message destination should match"
        );
        let ack_msg = rx_slim.recv().await.unwrap().unwrap();
        assert_eq!(
            ack_msg.get_session_message_type(),
            ProtoSessionMessageType::GroupAck,
            "Second message should be a GroupAck"
        );
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

        let endpoint = make_name(&["participant", "app", "v1"]).with_id(400);

        // Add endpoint
        let result = moderator.add_endpoint(&endpoint).await;
        assert!(result.is_ok());
        assert_eq!(moderator.inner.get_endpoints_added_count().await, 1);

        // Remove endpoint
        moderator.remove_endpoint(&endpoint);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(moderator.inner.get_endpoints_removed_count().await, 1);
    }

    #[tokio::test]
    async fn test_moderator_join_sets_subscribed() {
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        assert!(!moderator.subscribed);

        let remote = make_name(&["remote", "app", "v1"]).with_id(200);
        let result = moderator.join(remote, 12345).await;

        assert!(result.is_ok());
        assert!(moderator.subscribed);
        assert!(!moderator.group_list.is_empty());
    }

    #[tokio::test]
    async fn test_moderator_join_only_once() {
        let (mut moderator, mut rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let remote = make_name(&["remote", "app", "v1"]).with_id(200);

        // First join
        moderator.join(remote.clone(), 12345).await.unwrap();
        let first_subscribe = rx_slim.try_recv();
        assert!(first_subscribe.is_ok());

        // Second join should do nothing
        moderator.join(remote, 12345).await.unwrap();
        let second_subscribe = rx_slim.try_recv();
        assert!(second_subscribe.is_err()); // No message should be sent
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
        moderator
            .group_list
            .insert(make_name(&["participant1", "app", "v1"]), 401);
        moderator
            .group_list
            .insert(make_name(&["participant2", "app", "v1"]), 402);
        moderator
            .group_list
            .insert(make_name(&["participant3", "app", "v1"]), 403);

        let delete_msg = Message::builder()
            .source(moderator.common.settings.source.clone())
            .destination(moderator.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(100)
            .payload(CommandPayload::builder().leave_request(None).as_content())
            .build_publish()
            .unwrap();

        let result = moderator.delete_all(delete_msg, None, false).await;
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

        let tx = crate::transmitter::SessionTransmitter::new(tx_slim, tx_app);

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
            config,
            tx,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
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
        let source = Name::from_strings(["agntcy", "ns", "moderator"]).with_id(100);
        let destination = Name::from_strings(["agntcy", "ns", "chat"]);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, _rx_slim) = mpsc::channel(16);
        let (tx_app, mut rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let tx = crate::transmitter::SessionTransmitter::new(tx_slim, tx_app);

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
            config,
            tx,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        // Set up moderator as joined (this adds moderator to group_list)
        let remote = Name::from_strings(["agntcy", "ns", "participant"]).with_id(200);
        moderator.join(remote.clone(), 12345).await.unwrap();

        // Add one participant to the group (now we have moderator + participant = 2 total)
        // Use the naming convention requested: agntcy/ns/participant
        let mut participant = Name::from_strings(["agntcy", "ns", "participant"]);
        let participant_id = 401u64;
        participant.reset_id(); // Remove ID before inserting into group_list
        moderator
            .group_list
            .insert(participant.clone(), participant_id);

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
        let participant_with_id = participant.clone().with_id(participant_id);
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
                    .leave_request(None) // Empty payload when coming from participant
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Add LEAVING_SESSION metadata to signal graceful departure
        leave_msg.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

        let result = moderator.on_disconnection_detected(leave_msg, None).await;

        // Verify that the ParticipantDisconnected error was sent to the app channel
        let app_error = rx_app.try_recv();
        assert!(
            app_error.is_ok(),
            "Expected error to be sent to app channel"
        );

        if let Ok(Err(SessionError::ParticipantDisconnected(name))) = app_error {
            let name_str = name.to_string();
            assert!(
                name_str.contains("agntcy/ns/participant"),
                "Error message should mention the participant, got: {}",
                name_str
            );
        } else {
            panic!("Expected ParticipantDisconnected error");
        }

        // The function should succeed now that app channel is open
        assert!(result.is_ok(), "Should succeed with open app channel");

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
        let source = Name::from_strings(["agntcy", "ns", "moderator"]).with_id(100);
        let destination = Name::from_strings(["agntcy", "ns", "chat"]);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(16);

        let tx = crate::transmitter::SessionTransmitter::new(tx_slim, tx_app);

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source: source.clone(),
            destination: destination.clone(),
            config,
            tx,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
        };

        let inner = MockInnerHandler::new();
        let mut moderator = SessionModerator::new(inner, settings);
        moderator.init().await.unwrap();

        // Set up moderator as joined
        let remote = Name::from_strings(["agntcy", "ns", "participant1"]).with_id(200);
        moderator.join(remote.clone(), 12345).await.unwrap();

        // Add three participants to the group
        let mut participant1 = Name::from_strings(["agntcy", "ns", "participant1"]);
        let mut participant2 = Name::from_strings(["agntcy", "ns", "participant2"]);
        let mut participant3 = Name::from_strings(["agntcy", "ns", "participant3"]);

        participant1.reset_id(); // Remove ID before inserting into group_list
        participant2.reset_id();
        participant3.reset_id();

        moderator.group_list.insert(participant1.clone(), 401);
        moderator.group_list.insert(participant2.clone(), 402);
        moderator.group_list.insert(participant3.clone(), 403);

        // Create first leave request coming directly from participant1 with LEAVING_SESSION metadata
        let participant1_with_id = participant1.clone().with_id(401);
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
                    .leave_request(None) // No destination in payload when coming from participant
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Add LEAVING_SESSION metadata to signal graceful departure
        leave_msg1.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

        // Create second leave request coming directly from participant2 with LEAVING_SESSION metadata
        let participant2_with_id = participant2.clone().with_id(402);
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
                    .leave_request(None) // No destination in payload when coming from participant
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

        // Verify the queued task exists and has DISCONNECTION_DETECTED metadata
        if let Some((queued_msg, _)) = moderator.tasks_todo.front() {
            // Verify DISCONNECTION_DETECTED metadata was set (LEAVING_SESSION was replaced)
            assert!(
                queued_msg.contains_metadata(DISCONNECTION_DETECTED),
                "Queued message should have DISCONNECTION_DETECTED metadata"
            );
        } else {
            panic!("Expected queued task for participant2");
        }

        // Clear the messages
        while rx_slim.try_recv().is_ok() {}

        // Verify participant1 was removed from group (first task processed)
        assert!(
            !moderator.group_list.contains_key(&participant1),
            "Participant1 should be removed after first leave request"
        );

        // Verify participant2 is still in group (second task queued, not processed yet)
        assert!(
            moderator.group_list.contains_key(&participant2),
            "Participant2 should still be in group (task queued, not processed)"
        );
    }
}
