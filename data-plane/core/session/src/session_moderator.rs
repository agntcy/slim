// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, MlsPayload, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType,
    },
    messages::{Name, utils::DELETE_GROUP, utils::SlimHeaderFlags},
};
use tokio::sync::Mutex;

use slim_mls::mls::Mls;
use tracing::{debug, error};

use crate::{
    common::{MessageDirection, SessionMessage},
    errors::SessionError,
    mls_state::{MlsModeratorState, MlsState},
    moderator_task::{AddParticipant, ModeratorTask, RemoveParticipant, TaskUpdate},
    session_controller::SessionControllerCommon,
    session_settings::SessionSettings,
    traits::{MessageHandler, Transmitter},
};

pub struct SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// Queue of tasks to be performed by the moderator
    tasks_todo: VecDeque<Message>,

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

    /// Closing status
    closing: bool,

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
            closing: false,
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
            } => {
                if message.get_session_message_type().is_command_message() {
                    self.process_control_message(message).await
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
                        .on_message(SessionMessage::OnMessage { message, direction })
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
                    // 1. create the right error message
                    let error_message = match self.current_task.as_ref().unwrap() {
                        ModeratorTask::Add(_) => "failed to add a participant to the group",
                        ModeratorTask::Remove(_) => "failed to remove a participant from the group",
                        ModeratorTask::Update(_) => "failed to update state of the participant",
                    };

                    // 2. notify the application
                    // TODO: revisit this part
                    if let Err(e) = self
                        .common
                        .settings
                        .tx
                        .send_to_app(Err(SessionError::ModeratorTask(error_message.to_string())))
                        .await
                    {
                        error!("failed to notify application: {}", e);
                    }

                    // 3. delete current task and pick a new one
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
            SessionMessage::StartDrain { grace_period_ms: _ } => todo!(),
            SessionMessage::DeleteSession { session_id: _ } => todo!(),
        }
    }

    async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        self.inner.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &Name) {
        self.inner.remove_endpoint(endpoint);
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Moderator-specific cleanup
        self.subscribed = false;
        if !self.closing {
            self.send_close_signal().await;
        }
        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await
    }
}

impl<P, V, I> SessionModerator<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    async fn process_control_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest => self.on_discovery_request(message).await,
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
                    return self.delete_all(message).await;
                }

                // if the message contains a payload and the name is the same as the
                // local one, call the delete all anyway
                if let Some(n) = message
                    .get_payload()
                    .unwrap()
                    .as_command_payload()?
                    .as_leave_request_payload()?
                    .destination
                    .as_ref()
                    && Name::from(n) == self.common.settings.source
                {
                    return self.delete_all(message).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::GroupProposal => todo!(),
            ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
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
    async fn on_discovery_request(&mut self, mut msg: Message) -> Result<(), SessionError> {
        debug!(%self.common.settings.id, "received discovery request");
        // the channel discovery starts a new participant invite.
        // process the request only if not busy
        if self.current_task.is_some() {
            debug!(
                "Moderator is busy. Add invite participant task to the list and process it later"
            );
            // if busy postpone the task and add it to the todo list
            self.tasks_todo.push_back(msg);
            return Ok(());
        }

        // now the moderator is busy
        debug!("Create AddParticipantMls task");
        self.current_task = Some(ModeratorTask::Add(AddParticipant::default()));

        // check if there is a destination name in the payload. If yes recreate the message
        // with the right destination and send it out
        let payload = msg.extract_discovery_request().map_err(|e| {
            SessionError::Processing(format!(
                "failed to extract discovery request payload: {}",
                e
            ))
        })?;

        let mut discovery = match &payload.destination {
            Some(dst_name) => {
                // set the route to forward the messages correctly
                // here we assume that the destination is reachable from the
                // same connection from where we got the message from the controller
                let dst = Name::from(dst_name);
                self.common.set_route(&dst, msg.get_incoming_conn()).await?;

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
        self.current_task.as_mut().unwrap().discovery_start(id)?;

        debug!(
            "send discovery request to {} with id {}",
            discovery.get_dst(),
            discovery.get_id()
        );
        // send the message
        self.common.send_with_timer(discovery).await
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
                self.common.settings.config.duration,
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

    async fn on_leave_request(&mut self, mut msg: Message) -> Result<(), SessionError> {
        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list
            debug!("Moderator is busy. Add  leave request task to the list and process it later");
            self.tasks_todo.push_back(msg);
            return Ok(());
        }

        self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::default()));

        // adjust the message according to the sender:
        // - if coming from the controller (destination in the payload) we need to modify source and destination
        // - if coming from the app (empty payload) we need to add the participant id to the destination
        let payload_destination = msg
            .extract_leave_request()
            .map_err(|e| {
                SessionError::Processing(format!("failed to extract leave request payload: {}", e))
            })?
            .destination
            .clone();

        // Determine the destination name (without ID) based on payload
        let dst_without_id = match payload_destination {
            Some(ref dst_name) => Name::from(dst_name),
            None => msg.get_dst(),
        };

        // Look up participant ID in group list
        let id = *self
            .group_list
            .get(&dst_without_id)
            .ok_or(SessionError::RemoveParticipant(
                "participant not found".to_string(),
            ))?;

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
                    let mls_content = state.remove_participant(&leave_message).await?;
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

    async fn delete_all(&mut self, _msg: Message) -> Result<(), SessionError> {
        debug!("receive a close channel message, send signals to all participants");
        // create tasks to remove each participant from the group
        // even if mls is enable we just send the leave message
        // in any case the group will be deleted so there is no need to
        // update the mls state, this will speed up the process
        self.closing = true;
        // remove mls state
        self.mls_state = None;
        // clear all pending tasks
        self.tasks_todo.clear();

        // Remove the local name from the participants list
        let mut local = self.common.settings.source.clone();
        local.reset_id();
        self.group_list.remove(&local);
        // Collect the participants first to avoid borrowing conflicts
        let participants: Vec<Name> = self.group_list.keys().cloned().collect();

        for p in participants {
            // here we use only p as destination name, the id will be
            // added later in the on_leave_request message
            let leave = self.common.create_control_message(
                &p,
                ProtoSessionMessageType::LeaveRequest,
                rand::random::<u32>(),
                CommandPayload::builder().leave_request(None).as_content(),
                false,
            )?;
            // append the task to the list
            self.tasks_todo.push_back(leave);
        }

        // try to pickup the first task
        match self.tasks_todo.pop_front() {
            Some(m) => self.on_leave_request(m).await,
            None => {
                self.send_close_signal().await;
                Ok(())
            }
        }
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
            // moderator is busy, nothing to do
            return Ok(());
        }

        // check if there is a pending task to process
        let msg = match self.tasks_todo.pop_front() {
            Some(m) => m,
            None => {
                // nothing else to do
                debug!("No tasks left to perform");

                // check if we need to close the session
                if self.closing {
                    self.send_close_signal().await;
                }

                // return
                return Ok(());
            }
        };

        debug!("Process a new task from the todo list");
        // Process the control message by calling on_message
        // Since this is a control message coming from our internal queue,
        // we use MessageDirection::North (coming from network/control plane)
        self.on_message(SessionMessage::OnMessage {
            message: msg,
            direction: MessageDirection::North,
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

    async fn send_close_signal(&mut self) {
        // TODO check if the senders/receivers are happy as well
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
            error!("an error occurred while signaling session close");
        }
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
