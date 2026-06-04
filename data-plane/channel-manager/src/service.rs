// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC service implementation for the Channel Manager.

use std::sync::Arc;
use std::time::Duration;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::{ProtoName, ProtoSessionType};
use slim_service::app::App;
use slim_session::completion_handle::CompletionHandle;
use slim_session::{SessionConfig, SessionError, MlsSettings};
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
    app: Arc<App<AuthProvider, AuthVerifier>>,
    conn_id: u64,
    sessions: Arc<SessionsList>,
}

impl ChannelManagerServer {
    /// Create a new server instance
    pub fn new(
        app: Arc<App<AuthProvider, AuthVerifier>>,
        conn_id: u64,
        sessions: Arc<SessionsList>,
    ) -> Self {
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

    /// Await a two-step session operation (call + completion).
    async fn await_session_op(
        op: Result<CompletionHandle, SessionError>,
        error_context: &str,
    ) -> Result<(), String> {
        match op {
            Ok(completion) => completion
                .await
                .map_err(|e| format!("{error_context}: {e}")),
            Err(e) => Err(format!("{error_context}: {e}")),
        }
    }

    async fn handle_create_channel(&self, req: CreateChannelRequest) -> CommandResponse {
        let channel_name = &req.channel_name;

        // Check if the channel already exists before doing expensive SLIM work
        if self.sessions.get_session(channel_name).await.is_some() {
            return self.error_response(format!("channel {channel_name} already exists"));
        }

        // Parse the channel name
        let name = match ProtoName::parse_name(channel_name) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid channel name: {e}"));
            }
        };

        // Create a new session for the channel
        let session_config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            mls_settings: if req.mls_enabled {
                Some(MlsSettings::default())
            } else {
                None
            },
            max_retries: Some(10),
            interval: Some(Duration::from_millis(1000)),
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let (session, completion) = match self.app.create_session(session_config, name, None).await
        {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create channel {channel_name}: {e}");
                return self.error_response(format!("failed to create channel {channel_name}"));
            }
        };

        if let Err(e) = completion.await {
            error!("Failed to create channel {channel_name}: {e}");
            return self.error_response(format!("failed to create channel {channel_name}"));
        }

        // Keep a handle for cleanup in case of race condition
        let session_handle = session.session_arc();

        // Atomically check-and-insert to avoid race conditions
        if self
            .sessions
            .add_session(channel_name.clone(), session)
            .await
            .is_err()
        {
            // Channel was created by another concurrent request — clean up
            if let Some(s) = session_handle
                && let Err(e) = self.app.delete_session(&s)
            {
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

        let participant_name = match ProtoName::parse_name(participant_name_str) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid participant name: {e}"));
            }
        };

        // Set route for the participant
        if let Err(e) = self.app.set_route(&participant_name, self.conn_id).await {
            return self.error_response(format!(
                "failed to set route for participant {participant_name_str}: {e}"
            ));
        }

        // Invite the participant
        let op = session.invite_participant(&participant_name).await;
        if let Err(msg) = Self::await_session_op(
            op,
            &format!(
                "failed to invite participant {participant_name_str} to channel {channel_name}"
            ),
        )
        .await
        {
            return self.error_response(msg);
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

        let participant_name = match ProtoName::parse_name(participant_name_str) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid participant name: {e}"));
            }
        };

        let op = session.remove_participant(&participant_name).await;
        if let Err(msg) = Self::await_session_op(
            op,
            &format!(
                "failed to remove participant {participant_name_str} from channel {channel_name}"
            ),
        )
        .await
        {
            return self.error_response(msg);
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

        let participants = match session.participants_list().await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_channels_response_empty() {
        let response = ListChannelsResponse {
            success: true,
            error_msg: None,
            channel_name: vec![],
        };
        assert!(response.success, "response should indicate success");
        assert!(
            response.error_msg.is_none(),
            "no error message for successful response"
        );
        assert!(
            response.channel_name.is_empty(),
            "channel list should be empty"
        );
    }

    #[test]
    fn test_list_channels_response_with_data() {
        let channels = vec![
            "org/namespace/channel1".to_string(),
            "org/namespace/channel2".to_string(),
            "org/namespace/channel3".to_string(),
        ];
        let response = ListChannelsResponse {
            success: true,
            error_msg: None,
            channel_name: channels.clone(),
        };
        assert!(response.success);
        assert_eq!(response.channel_name.len(), 3);
        assert_eq!(response.channel_name, channels);
    }

    #[test]
    fn test_list_channels_response_error() {
        let response = ListChannelsResponse {
            success: false,
            error_msg: Some("internal server error".to_string()),
            channel_name: vec![],
        };
        assert!(!response.success, "response should indicate failure");
        assert_eq!(
            response.error_msg,
            Some("internal server error".to_string())
        );
        assert!(response.channel_name.is_empty());
    }

    #[test]
    fn test_list_participants_response_success() {
        let participants = vec!["org/ns/app1".to_string(), "org/ns/app2".to_string()];
        let response = ListParticipantsResponse {
            success: true,
            error_msg: None,
            participant_name: participants.clone(),
        };
        assert!(response.success);
        assert!(response.error_msg.is_none());
        assert_eq!(response.participant_name, participants);
    }

    #[test]
    fn test_list_participants_response_empty() {
        let response = ListParticipantsResponse {
            success: true,
            error_msg: None,
            participant_name: vec![],
        };
        assert!(response.success);
        assert!(response.participant_name.is_empty());
    }

    #[test]
    fn test_list_participants_response_channel_not_found() {
        let response = ListParticipantsResponse {
            success: false,
            error_msg: Some("channel not found".to_string()),
            participant_name: vec![],
        };
        assert!(!response.success);
        assert_eq!(response.error_msg, Some("channel not found".to_string()));
        assert!(response.participant_name.is_empty());
    }

    #[test]
    fn test_list_participants_response_query_failed() {
        let response = ListParticipantsResponse {
            success: false,
            error_msg: Some("failed to query participants".to_string()),
            participant_name: vec![],
        };
        assert!(!response.success);
        assert!(response.error_msg.is_some());
        assert!(response.error_msg.unwrap().contains("failed"));
    }

    #[test]
    fn test_command_response_success() {
        let response = CommandResponse {
            success: true,
            error_msg: None,
        };
        assert!(response.success, "success field should be true");
        assert!(
            response.error_msg.is_none(),
            "error_msg should be None for success"
        );
    }

    #[test]
    fn test_command_response_error() {
        let response = CommandResponse {
            success: false,
            error_msg: Some("operation failed".to_string()),
        };
        assert!(!response.success, "success field should be false");
        assert_eq!(
            response.error_msg,
            Some("operation failed".to_string()),
            "error_msg should contain the error"
        );
    }

    #[test]
    fn test_command_response_channel_exists() {
        let response = CommandResponse {
            success: false,
            error_msg: Some("channel org/ns/ch already exists".to_string()),
        };
        assert!(!response.success);
        assert!(
            response
                .error_msg
                .as_ref()
                .unwrap()
                .contains("already exists"),
            "error message should indicate the channel exists"
        );
    }

    #[test]
    fn test_command_response_invalid_name() {
        let response = CommandResponse {
            success: false,
            error_msg: Some("invalid channel name: invalid/format".to_string()),
        };
        assert!(!response.success);
        assert!(
            response
                .error_msg
                .as_ref()
                .unwrap()
                .contains("invalid channel name"),
            "error message should mention invalid name"
        );
    }

    #[test]
    fn test_create_channel_request() {
        let request = CreateChannelRequest {
            channel_name: "org/namespace/channel".to_string(),
            mls_enabled: true,
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert!(request.mls_enabled);
    }

    #[test]
    fn test_create_channel_request_mls_disabled() {
        let request = CreateChannelRequest {
            channel_name: "org/namespace/channel".to_string(),
            mls_enabled: false,
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert!(!request.mls_enabled);
    }

    #[test]
    fn test_delete_channel_request() {
        let request = DeleteChannelRequest {
            channel_name: "org/namespace/channel".to_string(),
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
    }

    #[test]
    fn test_add_participant_request() {
        let request = AddParticipantRequest {
            channel_name: "org/namespace/channel".to_string(),
            participant_name: "org/namespace/app".to_string(),
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert_eq!(request.participant_name, "org/namespace/app");
    }

    #[test]
    fn test_delete_participant_request() {
        let request = DeleteParticipantRequest {
            channel_name: "org/namespace/channel".to_string(),
            participant_name: "org/namespace/app".to_string(),
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert_eq!(request.participant_name, "org/namespace/app");
    }

    #[test]
    fn test_list_participants_request() {
        let request = ListParticipantsRequest {
            channel_name: "org/namespace/channel".to_string(),
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
    }

    #[test]
    fn test_response_error_message_formatting() {
        let channel = "org/ns/channel";
        let error_msg = format!("channel {channel} not found");
        let response = CommandResponse {
            success: false,
            error_msg: Some(error_msg),
        };
        assert_eq!(
            response.error_msg,
            Some("channel org/ns/channel not found".to_string())
        );
    }

    #[test]
    fn test_list_participants_response_multiple_participants() {
        let participants = vec![
            "org/ns/app1".to_string(),
            "org/ns/app2".to_string(),
            "org/ns/app3".to_string(),
            "org/ns/app4".to_string(),
        ];
        let response = ListParticipantsResponse {
            success: true,
            error_msg: None,
            participant_name: participants.clone(),
        };
        assert_eq!(response.participant_name.len(), 4);
        assert!(
            response
                .participant_name
                .contains(&"org/ns/app1".to_string())
        );
        assert!(
            response
                .participant_name
                .contains(&"org/ns/app4".to_string())
        );
    }
}
