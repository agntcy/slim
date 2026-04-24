// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC service implementation for the Channel Manager.

use std::sync::Arc;
use std::time::Duration;

use slim_bindings::{App, Name, SessionConfig, SessionType};
use tonic::{Request, Response, Status};
use tracing::{error, info};

use crate::proto::channel_manager_service_server::ChannelManagerService;
use crate::proto::{
    AddParticipantRequest, CommandResponse, ControlRequest, ControlResponse,
    CreateChannelRequest, DeleteChannelRequest, DeleteParticipantRequest,
    ListChannelsRequest, ListChannelsResponse, ListParticipantsRequest,
    ListParticipantsResponse,
    control_request::Payload as RequestPayload,
    control_response::Payload as ResponsePayload,
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

    fn success_response(&self, msg_id: u64) -> ControlResponse {
        ControlResponse {
            msg_id,
            payload: Some(ResponsePayload::CommandResponse(CommandResponse {
                msg_id,
                success: true,
                error_msg: None,
            })),
        }
    }

    fn error_response(&self, msg_id: u64, error_msg: String) -> ControlResponse {
        ControlResponse {
            msg_id,
            payload: Some(ResponsePayload::CommandResponse(CommandResponse {
                msg_id,
                success: false,
                error_msg: Some(error_msg),
            })),
        }
    }

    async fn handle_create_channel(
        &self,
        msg_id: u64,
        req: CreateChannelRequest,
    ) -> ControlResponse {
        let channel_name = &req.channel_name;

        // Parse the channel name
        let name = match Name::from_string(channel_name.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(msg_id, format!("invalid channel name: {e}"));
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
                return self
                    .error_response(msg_id, format!("failed to create channel {channel_name}"));
            }
        };

        // Atomically check-and-insert to avoid race conditions
        if !self
            .sessions
            .try_insert_session(channel_name.clone(), session.clone())
            .await
        {
            // Channel was created by another request — clean up the session we just created
            if let Err(e) = self.app.delete_session_and_wait_async(session).await {
                error!("Failed to clean up duplicate session for {channel_name}: {e}");
            }
            return self.error_response(msg_id, format!("channel {channel_name} already exists"));
        }

        info!("Created channel {channel_name}");
        self.success_response(msg_id)
    }

    async fn handle_delete_channel(
        &self,
        msg_id: u64,
        req: DeleteChannelRequest,
    ) -> ControlResponse {
        let channel_name = &req.channel_name;

        let session = match self.sessions.remove_session(channel_name).await {
            Some(s) => s,
            None => {
                return self
                    .error_response(msg_id, format!("channel {channel_name} not found"));
            }
        };

        if let Err(e) = self.app.delete_session_and_wait_async(session).await {
            error!("Failed to delete channel {channel_name}: {e}");
            return self
                .error_response(msg_id, format!("failed to delete channel {channel_name}: {e}"));
        }

        info!("Deleted channel {channel_name}");
        self.success_response(msg_id)
    }

    async fn handle_add_participant(
        &self,
        msg_id: u64,
        req: AddParticipantRequest,
    ) -> ControlResponse {
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self
                    .error_response(msg_id, format!("channel {channel_name} not found"));
            }
        };

        let participant_name = match Name::from_string(participant_name_str.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(
                    msg_id,
                    format!("invalid participant name: {e}"),
                );
            }
        };

        // Set route for the participant
        if let Err(e) = self
            .app
            .set_route_async(Arc::new(participant_name.clone()), self.conn_id)
            .await
        {
            return self.error_response(
                msg_id,
                format!("failed to set route for participant {participant_name_str}: {e}"),
            );
        }

        // Invite the participant
        if let Err(e) = session
            .invite_and_wait_async(Arc::new(participant_name))
            .await
        {
            return self.error_response(
                msg_id,
                format!(
                    "failed to invite participant {participant_name_str} to channel {channel_name}: {e}"
                ),
            );
        }

        info!(
            "Added participant {participant_name_str} to channel {channel_name}"
        );
        self.success_response(msg_id)
    }

    async fn handle_delete_participant(
        &self,
        msg_id: u64,
        req: DeleteParticipantRequest,
    ) -> ControlResponse {
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self
                    .error_response(msg_id, format!("channel {channel_name} not found"));
            }
        };

        let participant_name = match Name::from_string(participant_name_str.clone()) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(
                    msg_id,
                    format!("invalid participant name: {e}"),
                );
            }
        };

        if let Err(e) = session
            .remove_and_wait_async(Arc::new(participant_name))
            .await
        {
            return self.error_response(
                msg_id,
                format!(
                    "failed to remove participant {participant_name_str} from channel {channel_name}: {e}"
                ),
            );
        }

        info!(
            "Removed participant {participant_name_str} from channel {channel_name}"
        );
        self.success_response(msg_id)
    }

    async fn handle_list_channels(&self, msg_id: u64, _req: ListChannelsRequest) -> ControlResponse {
        let channels = self.sessions.list_channel_names().await;
        info!("Listing channels, count: {}", channels.len());

        ControlResponse {
            msg_id,
            payload: Some(ResponsePayload::ListChannelsResponse(
                ListChannelsResponse {
                    msg_id,
                    channel_name: channels,
                },
            )),
        }
    }

    async fn handle_list_participants(
        &self,
        msg_id: u64,
        req: ListParticipantsRequest,
    ) -> ControlResponse {
        let channel_name = &req.channel_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self
                    .error_response(msg_id, format!("channel {channel_name} not found"));
            }
        };

        let participants = match session.participants_list_async().await {
            Ok(p) => p,
            Err(e) => {
                return self.error_response(
                    msg_id,
                    format!("failed to list participants for channel {channel_name}: {e}"),
                );
            }
        };

        let participant_names: Vec<String> = participants.iter().map(|p| p.to_string()).collect();

        info!(
            "Listing participants for channel {channel_name}, count: {}",
            participant_names.len()
        );

        ControlResponse {
            msg_id,
            payload: Some(ResponsePayload::ListParticipantsResponse(
                ListParticipantsResponse {
                    msg_id,
                    participant_name: participant_names,
                },
            )),
        }
    }
}

#[tonic::async_trait]
impl ChannelManagerService for ChannelManagerServer {
    async fn command(
        &self,
        request: Request<ControlRequest>,
    ) -> Result<Response<ControlResponse>, Status> {
        let req = request.into_inner();
        let msg_id = req.msg_id;

        info!("Received command, msg_id: {msg_id}");

        let response = match req.payload {
            Some(RequestPayload::CreateChannelRequest(r)) => {
                self.handle_create_channel(msg_id, r).await
            }
            Some(RequestPayload::DeleteChannelRequest(r)) => {
                self.handle_delete_channel(msg_id, r).await
            }
            Some(RequestPayload::AddParticipantRequest(r)) => {
                self.handle_add_participant(msg_id, r).await
            }
            Some(RequestPayload::DeleteParticipantRequest(r)) => {
                self.handle_delete_participant(msg_id, r).await
            }
            Some(RequestPayload::ListChannelsRequest(r)) => {
                self.handle_list_channels(msg_id, r).await
            }
            Some(RequestPayload::ListParticipantsRequest(r)) => {
                self.handle_list_participants(msg_id, r).await
            }
            None => {
                return Err(Status::invalid_argument("missing payload"));
            }
        };

        Ok(Response::new(response))
    }
}
