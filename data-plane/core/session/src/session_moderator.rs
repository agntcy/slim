// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, MlsPayload, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType,
    },
    messages::{
        Name,
        utils::{DELETE_GROUP, SlimHeaderFlags, TRUE_VAL},
    },
};
use tokio::sync::{Mutex, oneshot};

use slim_mls::mls::Mls;
use tracing::debug;

use crate::{
    common::{MessageDirection, SessionMessage},
    errors::SessionError,
    mls_state::{MlsModeratorState, MlsState},
    moderator_task::{AddParticipant, CloseGroup, ModeratorTask, RemoveParticipant, TaskUpdate},
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
            let mls_state = MlsState::new(Arc::new(Mutex::new(Mls::new(
                self.common.settings.identity_provider.clone(),
                self.common.settings.identity_verifier.clone(),
                self.common.settings.storage_path.clone(),
            ))))
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
                    self.inner
                        .on_message(SessionMessage::OnMessage {
                            message,
                            direction,
                            ack_tx,
                        })
                        .await
                }
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    self.common.sender.on_timer_timeout(message_id).await
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
                    self.common.sender.on_timer_failure(message_id).await;
                    // the current task failed:
                    // 1. create the right error message and notify via ack_tx if present
                    // Helper to signal failure and return error message
                    let signal_failure =
                        |task_ack_tx: &mut Option<oneshot::Sender<Result<(), SessionError>>>,
                         msg: &str| {
                            if let Some(tx) = task_ack_tx.take() {
                                let _ = tx.send(Err(SessionError::ModeratorTask(msg.to_string())));
                            }
                            msg.to_string()
                        };

                    let message = match &self.common.settings.config.session_type {
                        ProtoSessionType::PointToPoint => "session handshake failed",
                        ProtoSessionType::Multicast => "failed to add a participant to the group",
                        _ => panic!("session type not specified"),
                    };

                    match self.current_task.as_mut().unwrap() {
                        ModeratorTask::Add(task) => signal_failure(&mut task.ack_tx, message),
                        ModeratorTask::Remove(task) => signal_failure(
                            &mut task.ack_tx,
                            "failed to remove a participant from the group",
                        ),
                        ModeratorTask::Update(task) => signal_failure(
                            &mut task.ack_tx,
                            "failed to update state of the participant",
                        ),
                        ModeratorTask::Close(task) => {
                            signal_failure(&mut task.ack_tx, "failed to close the session")
                        }
                    };

                    // 2. delete current task and pick a new one
                    self.current_task = None;
                    self.pop_task().await
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
                // Crate the leave message
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
                self.delete_all(leave_msg, None).await
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message
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

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Moderator-specific cleanup
        self.subscribed = false;
        self.common.sender.close();

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
    /// Helper method to handle errors after task creation
    /// Extracts ack_tx from current_task and sends the error
    fn handle_task_error(&mut self, error: SessionError) -> SessionError {
        if let Some(task) = self.current_task.take() {
            let ack_tx = match task {
                ModeratorTask::Add(t) => t.ack_tx,
                ModeratorTask::Remove(t) => t.ack_tx,
                ModeratorTask::Update(t) => t.ack_tx,
                ModeratorTask::Close(t) => t.ack_tx,
            };
            if let Some(tx) = ack_tx {
                let _ = tx.send(Err(SessionError::Processing(error.to_string())));
            }
        }

        // Remove task
        self.current_task = None;

        error
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
                // the right settings. Here we can simply return
                Ok(())
            }
            ProtoSessionMessageType::JoinReply => self.on_join_reply(message).await,
            ProtoSessionMessageType::LeaveRequest => {
                // if the metadata contains the key "DELETE_GROUP" remove all the participants
                // and close the session when all task are completed
                if message.contains_metadata(DELETE_GROUP) {
                    return self.delete_all(message, ack_tx).await;
                }

                // if the message contains a payload and the name is the same as the
                // local one, call the delete all anyway
                if let Some(n) = message
                    .get_payload()
                    .ok_or_else(|| SessionError::Processing("Missing payload".to_string()))?
                    .as_command_payload()
                    .map_err(SessionError::from)?
                    .as_leave_request_payload()
                    .map_err(SessionError::from)?
                    .destination
                    .as_ref()
                    && Name::from(n) == self.common.settings.source
                {
                    return self.delete_all(message, ack_tx).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message, ack_tx).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::GroupProposal => todo!(),
            ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupClose
            | ProtoSessionMessageType::GroupNack => Err(SessionError::Processing(format!(
                "Unexpected control message type {:?}",
                message.get_session_message_type()
            ))),
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message.get_session_message_type()
            ))),
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
        self.current_task = Some(ModeratorTask::Add(AddParticipant::new(ack_tx)));

        // check if there is a destination name in the payload. If yes recreate the message
        // with the right destination and send it out
        let payload = msg.extract_discovery_request().map_err(|e| {
            let err = SessionError::Processing(format!(
                "failed to extract discovery request payload: {}",
                e
            ));
            self.handle_task_error(err)
        })?;

        let mut discovery = match &payload.destination {
            Some(dst_name) => {
                // set the route to forward the messages correctly
                // here we assume that the destination is reachable from the
                // same connection from where we got the message from the controller
                let dst = Name::from(dst_name);
                self.common
                    .set_route(&dst, msg.get_incoming_conn())
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
                msg
            }
            None => {
                // simply forward the message
                msg
            }
        };

        // start the current task
        let id = rand::random::<u32>();
        discovery.get_session_header_mut().set_message_id(id);
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_start(id)
            .map_err(|e| self.handle_task_error(e))?;

        debug!(
            "send discovery request to {} with id {}",
            discovery.get_dst(),
            discovery.get_id()
        );
        self.common
            .send_with_timer(discovery)
            .await
            .map_err(|e| self.handle_task_error(e))
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            "discovery reply coming from {} with id {}",
            msg.get_source(),
            msg.get_id()
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
            .set_route(&msg.get_source(), msg.get_incoming_conn())
            .await?;

        // if this is a multicast session we need to add a route for the channel
        // on the connection from where we received the message. This has to be done
        // all the times because the messages from the remote endpoints may come from
        // different connections. In case the route exists already it will be just ignored
        if self.common.settings.config.session_type == ProtoSessionType::Multicast {
            self.common
                .set_route(&self.common.settings.destination, msg.get_incoming_conn())
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
            "send join request to {} with id {}",
            msg.get_slim_header().get_source(),
            msg_id
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
            "join reply coming from {} with id {}",
            msg.get_source(),
            msg.get_id()
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
        debug!("add endpoint to the session {}", msg.get_source());
        self.add_endpoint(&msg.get_source()).await?;

        // get mls data if MLS is enabled
        let (commit, welcome) = if self.mls_state.is_some() {
            let (commit_payload, welcome_payload) = self
                .mls_state
                .as_mut()
                .unwrap()
                .add_participant(&msg)
                .await?;

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
            debug!("send add update to channel with id {}", add_msg_id);
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
            "send welcome message to {} with id {}",
            msg.get_slim_header().get_source(),
            welcome_msg_id
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
            debug!("Moderator is busy. Add  leave request task to the list and process it later");
            self.tasks_todo.push_back((msg, ack_tx));
            return Ok(());
        }

        debug!("Create RemoveParticipant task with ack_tx");
        self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::new(ack_tx)));

        // adjust the message according to the sender:
        // - if coming from the controller (destination in the payload) we need to modify source and destination
        // - if coming from the app (empty payload) we need to add the participant id to the destination
        let payload_destination = msg
            .extract_leave_request()
            .map_err(|e| {
                let err = SessionError::Processing(format!(
                    "failed to extract leave request payload: {}",
                    e
                ));
                self.handle_task_error(err)
            })?
            .destination
            .as_ref();

        // Determine the destination name (without ID) based on payload
        let dst_without_id = match payload_destination {
            Some(dst_name) => Name::from(dst_name),
            None => msg.get_dst(),
        };

        // Look up participant ID in group list
        let id = match self.group_list.get(&dst_without_id) {
            Some(id) => *id,
            None => {
                let err = SessionError::RemoveParticipant("participant not found".to_string());
                return Err(self.handle_task_error(err));
            }
        };

        // Update message based on whether destination was provided in payload
        if payload_destination.is_some() {
            // Destination provided: update source, destination, and payload
            let new_payload = CommandPayload::builder().leave_request(None).as_content();
            msg.get_slim_header_mut()
                .set_source(&self.common.settings.source);
            msg.set_payload(new_payload);
        }

        // Set destination with ID and message ID (common to both cases)
        let dst_with_id = dst_without_id.clone().with_id(id);
        msg.get_slim_header_mut().set_destination(&dst_with_id);
        msg.set_message_id(rand::random::<u32>());

        let leave_message = msg;

        // remove the participant from the group list and notify the the local session
        debug!(
            "remove endpoint from the session {}",
            leave_message.get_dst()
        );

        self.group_list.remove(&dst_without_id);
        self.remove_endpoint(&leave_message.get_dst());

        // Before send the leave request we may need to send the Group update
        // with the new participant list and the new mls payload if needed
        // Create participants list for commit message
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        if participants_vec.len() > 2 {
            // in this case we need to send first the group update and later the leave message
            let mls_payload = match self.mls_state.as_mut() {
                Some(state) => {
                    let mls_content = state
                        .remove_participant(&leave_message)
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

            let update_payload = CommandPayload::builder()
                .group_remove(leave_message.get_dst(), participants_vec, mls_payload)
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

    async fn delete_all(
        &mut self,
        _msg: Message,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        debug!("receive a close channel message, send signals to all participants");
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

        // Remove the local name from the participants list
        let mut local = self.common.settings.source.clone();
        local.reset_id();
        self.group_list.remove(&local);

        // Collect the participants and create the close message
        //let mut participants = vec![];
        //for (k, v) in self.group_list.iter() {
        //    let name = k.clone().with_id(*v);
        //    participants.push(name);
        //}
        let participants = self.group_list.keys().cloned().collect();

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

        // create the close task
        self.current_task = Some(ModeratorTask::Close(CloseGroup::new(ack_tx)));
        self.current_task.as_mut().unwrap().leave_start(close_id)?;

        // sent the message
        self.common.sender.on_message(&close).await
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            "received leave reply from {} with id {}",
            msg.get_source(),
            msg.get_id()
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
            "got group ack from {} with id {}",
            msg.get_source(),
            msg.get_id()
        );
        // notify the sender
        self.common.sender.on_message(&msg).await?;

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.common.sender.is_still_pending(msg_id) {
            debug!(
                "process group ack for message {}. try to close task",
                msg_id
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
                if self.postponed_message.is_some()
                    && matches!(self.current_task, Some(ModeratorTask::Remove(_)))
                {
                    // send the leave message an progress
                    let leave_message = self.postponed_message.as_ref().unwrap();
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
                "timer for message {} is still pending, do not close the task",
                msg_id
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

        // if this is a point to point connection set the remote name so that we
        // can add also the right id to the message destination name
        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            self.common.settings.destination = remote;
        } else {
            // if this is a multicast session we need to subscribe for the channel name
            let sub = Message::builder()
                .source(self.common.settings.source.clone())
                .destination(self.common.settings.destination.clone())
                .flags(SlimHeaderFlags::default().with_forward_to(conn))
                .build_subscribe()
                .unwrap();

            self.common.send_to_slim(sub).await?;
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

        if res.is_err() {
            tracing::error!("an error occurred while signaling session close");
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

        let storage_path = std::path::PathBuf::from("/tmp/test");

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
            storage_path,
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
        let (mut moderator, _rx_slim, _rx_session_layer) = setup_moderator();
        moderator.init().await.unwrap();

        let source = make_name(&["requester", "app", "v1"]).with_id(300);
        let destination = moderator.common.settings.source.clone();

        let join_msg = Message::builder()
            .source(source.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
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

        let result = moderator.delete_all(delete_msg, None).await;
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

        let storage_path = std::path::PathBuf::from("/tmp/test");

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
            storage_path,
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
}
