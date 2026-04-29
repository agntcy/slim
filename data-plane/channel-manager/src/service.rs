// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC service implementation for the Channel Manager.

use std::sync::Arc;
use std::time::Duration;

use slim_bindings::{App, Name, SessionConfig, SessionType};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::proto::channel_manager_service_server::ChannelManagerService;
use crate::proto::{
    AddParticipantRequest, CommandResponse, CreateChannelRequest, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListChannelsResponse, ListParticipantsRequest,
    ListParticipantsResponse,
};
use crate::sessions::SessionsList;

/// gRPC server for the Channel Manager service
pub struct ChannelManagerServer {
    app: Arc<App>,
    conn_id: u64,
    sessions: Arc<SessionsList>,
}

impl ChannelManagerServer {
    /// Create a new server instance
    pub fn new(app: Arc<App>, conn_id: u64, sessions: Arc<SessionsList>) -> Self {
        Self {
            app,
            conn_id,
            sessions,
        }
    }

    fn success_response(&self) -> CommandResponse {
        CommandResponse {
            success: true,
            error_msg: None,
        }
    }

    fn error_response(&self, error_msg: String) -> CommandResponse {
        CommandResponse {
            success: false,
            error_msg: Some(error_msg),
        }
    }

    async fn handle_create_channel(&self, req: CreateChannelRequest) -> CommandResponse {
        let channel_name = &req.channel_name;

        // Check if the channel already exists before doing expensive SLIM work
        if self.sessions.get_session(channel_name).await.is_some() {
            return self.error_response(format!("channel {channel_name} already exists"));
        }

        // Parse the channel name
        let name = match Name::from_string(channel_name.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid channel name: {e}"));
            }
        };

        // Create a new session for the channel
        let session_config = SessionConfig {
            session_type: SessionType::Group,
            enable_mls: req.mls_enabled,
            max_retries: Some(10),
            interval: Some(Duration::from_millis(1000)),
            metadata: std::collections::HashMap::new(),
        };

        let session = match self
            .app
            .create_session_and_wait_async(session_config, Arc::new(name))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create channel {channel_name}: {e}");
                return self.error_response(format!("failed to create channel {channel_name}"));
            }
        };

        // Atomically check-and-insert to avoid race conditions
        if !self
            .sessions
            .try_insert_session(channel_name.clone(), session.clone())
            .await
        {
            // Channel was created by another concurrent request — clean up
            if let Err(e) = self.app.delete_session_and_wait_async(session).await {
                error!("Failed to clean up duplicate session for {channel_name}: {e}");
            }
            return self.error_response(format!("channel {channel_name} already exists"));
        }

        info!("Created channel {channel_name}");
        self.success_response()
    }

    async fn handle_delete_channel(&self, req: DeleteChannelRequest) -> CommandResponse {
        let channel_name = &req.channel_name;

        if let Err(e) = self.sessions.remove_session(channel_name, &self.app).await {
            error!("Failed to delete channel {channel_name}: {e}");
            return self.error_response(format!("{e}"));
        }

        info!("Deleted channel {channel_name}");
        self.success_response()
    }

    async fn handle_add_participant(&self, req: AddParticipantRequest) -> CommandResponse {
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self.error_response(format!("channel {channel_name} not found"));
            }
        };

        let participant_name = match Name::from_string(participant_name_str.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid participant name: {e}"));
            }
        };

        // Set route for the participant
        if let Err(e) = self
            .app
            .set_route_async(Arc::new(participant_name.clone()), self.conn_id)
            .await
        {
            return self.error_response(format!(
                "failed to set route for participant {participant_name_str}: {e}"
            ));
        }

        // Invite the participant
        if let Err(e) = session
            .invite_and_wait_async(Arc::new(participant_name))
            .await
        {
            return self.error_response(format!(
                "failed to invite participant {participant_name_str} to channel {channel_name}: {e}"
            ));
        }

        info!("Added participant {participant_name_str} to channel {channel_name}");
        self.success_response()
    }

    async fn handle_delete_participant(&self, req: DeleteParticipantRequest) -> CommandResponse {
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self.error_response(format!("channel {channel_name} not found"));
            }
        };

        let participant_name = match Name::from_string(participant_name_str.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid participant name: {e}"));
            }
        };

        if let Err(e) = session
            .remove_and_wait_async(Arc::new(participant_name))
            .await
        {
            return self.error_response(
                format!(
                    "failed to remove participant {participant_name_str} from channel {channel_name}: {e}"
                ),
            );
        }

        info!("Removed participant {participant_name_str} from channel {channel_name}");
        self.success_response()
    }

    async fn handle_list_channels(&self) -> ListChannelsResponse {
        let channels = self.sessions.list_channel_names().await;
        info!("Listing channels, count: {}", channels.len());

        ListChannelsResponse {
            success: true,
            error_msg: None,
            channel_name: channels,
        }
    }

    async fn handle_list_participants(
        &self,
        req: ListParticipantsRequest,
    ) -> ListParticipantsResponse {
        let channel_name = &req.channel_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return ListParticipantsResponse {
                    success: false,
                    error_msg: Some(format!("channel {channel_name} not found")),
                    participant_name: vec![],
                };
            }
        };

        let participants = match session.participants_list_async().await {
            Ok(p) => p,
            Err(e) => {
                return ListParticipantsResponse {
                    success: false,
                    error_msg: Some(format!(
                        "failed to list participants for channel {channel_name}: {e}"
                    )),
                    participant_name: vec![],
                };
            }
        };

        let participant_names: Vec<String> = participants.iter().map(|p| p.to_string()).collect();

        info!(
            "Listing participants for channel {channel_name}, count: {}",
            participant_names.len()
        );

        ListParticipantsResponse {
            success: true,
            error_msg: None,
            participant_name: participant_names,
        }
    }
}

#[tonic::async_trait]
impl ChannelManagerService for ChannelManagerServer {
    async fn create_channel(
        &self,
        request: Request<CreateChannelRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let req = request.into_inner();
        debug!("Received create_channel request");
        Ok(Response::new(self.handle_create_channel(req).await))
    }

    async fn delete_channel(
        &self,
        request: Request<DeleteChannelRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let req = request.into_inner();
        debug!("Received delete_channel request");
        Ok(Response::new(self.handle_delete_channel(req).await))
    }

    async fn add_participant(
        &self,
        request: Request<AddParticipantRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let req = request.into_inner();
        debug!("Received add_participant request");
        Ok(Response::new(self.handle_add_participant(req).await))
    }

    async fn delete_participant(
        &self,
        request: Request<DeleteParticipantRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let req = request.into_inner();
        debug!("Received delete_participant request");
        Ok(Response::new(self.handle_delete_participant(req).await))
    }

    async fn list_channels(
        &self,
        _request: Request<ListChannelsRequest>,
    ) -> Result<Response<ListChannelsResponse>, Status> {
        debug!("Received list_channels request");
        Ok(Response::new(self.handle_list_channels().await))
    }

    async fn list_participants(
        &self,
        request: Request<ListParticipantsRequest>,
    ) -> Result<Response<ListParticipantsResponse>, Status> {
        let req = request.into_inner();
        debug!("Received list_participants request");
        Ok(Response::new(self.handle_list_participants(req).await))
    }
}
