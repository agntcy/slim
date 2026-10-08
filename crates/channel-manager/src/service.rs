// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC service implementation for the Channel Manager.

use std::sync::Arc;
use std::time::Duration;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::{ProtoName, ProtoSessionType};
use slim_service::app::App;
use slim_session::completion_handle::CompletionHandle;
use slim_session::{SessionConfig, SessionError, session_config::MlsSettings};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::caller_identity::CallerIdentity;
use crate::grant::GrantVerifier;
use crate::ownership::ChannelOwnership;
use crate::proto::channel_manager_service_server::ChannelManagerService;
use crate::proto::{
    AddParticipantRequest, ChannelInfo, CommandResponse, CreateChannelRequest,
    DeleteChannelRequest, DeleteParticipantRequest, ListChannelsRequest, ListChannelsResponse,
    ListParticipantsRequest, ListParticipantsResponse,
};
use crate::sessions::SessionsList;

/// Enforces the grant contract for a mutating participant-change RPC.
///
/// A channel with no owner on record needs no grant -- unchanged,
/// pre-ownership behavior. The owner needs no grant for their own channel.
/// Anyone else must present one that verifies and names this exact channel
/// and participant and hasn't expired.
///
/// A free function (not a method) so it's testable without constructing a
/// full `ChannelManagerServer`, which needs a real `App`.
async fn enforce_grant(
    ownership: &ChannelOwnership,
    grant_verifier: &dyn GrantVerifier,
    channel_name: &str,
    participant_name: &str,
    caller: &Option<CallerIdentity>,
    grant: &Option<Vec<u8>>,
) -> Result<(), String> {
    let Some(owner) = ownership.get_owner(channel_name).await else {
        return Ok(());
    };
    if caller.as_ref().is_some_and(|c| c.subject == owner) {
        return Ok(());
    }

    let Some(grant_bytes) = grant else {
        return Err(format!(
            "channel {channel_name} requires a grant from its owner"
        ));
    };

    let parsed = grant_verifier
        .verify(&owner, grant_bytes)
        .map_err(|e| format!("invalid grant: {e}"))?;

    if parsed.channel != channel_name || parsed.invitee != participant_name {
        return Err("grant does not authorize this channel/participant".to_string());
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if parsed.not_after < now {
        return Err("grant has expired".to_string());
    }

    Ok(())
}

/// gRPC server for the Channel Manager service
pub struct ChannelManagerServer {
    app: Arc<App<AuthProvider, AuthVerifier>>,
    sessions: Arc<SessionsList>,
    ownership: ChannelOwnership,
    grant_verifier: Arc<dyn GrantVerifier>,
    /// When true, channels are owned by the config file and mutating APIs are disabled.
    config_mode: bool,
}

impl ChannelManagerServer {
    /// Create a new server instance.
    ///
    /// Set `config_mode` to `true` when the config file defines channels; this
    /// makes all write operations return `FAILED_PRECONDITION` so the config
    /// file remains the single source of truth.
    ///
    /// `conn_id` pins the default gateway, so invites keep working on a
    /// multi-uplink service where auto-detection would be ambiguous.
    ///
    /// `grant_verifier` authenticates a grant presented by a non-owner
    /// caller on `AddParticipant`/`DeleteParticipant` -- swap in a different
    /// implementation for a different grant format or key scheme (see
    /// `crate::grant::GrantVerifier`); `DidKeyEd25519Verifier` is a
    /// reasonable default.
    pub fn new(
        app: Arc<App<AuthProvider, AuthVerifier>>,
        conn_id: u64,
        sessions: Arc<SessionsList>,
        config_mode: bool,
        grant_verifier: Arc<dyn GrantVerifier>,
    ) -> Self {
        if let Err(e) = app.set_default_gateway(conn_id) {
            warn!("failed to pin default gateway to conn {conn_id}: {e}");
        }

        Self {
            app,
            sessions,
            ownership: ChannelOwnership::new(),
            grant_verifier,
            config_mode,
        }
    }

    /// Guard for mutating operations: returns an error response when the server
    /// is running in config mode (channels are owned by the config file).
    fn ensure_api_mode(&self) -> Option<CommandResponse> {
        if self.config_mode {
            Some(
                self.error_response(
                    "channel manager is running in config mode; \
                 modify the config file and restart to change channels or participants"
                        .to_string(),
                ),
            )
        } else {
            None
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

    async fn handle_create_channel(
        &self,
        req: CreateChannelRequest,
        caller: Option<CallerIdentity>,
    ) -> CommandResponse {
        if let Some(err) = self.ensure_api_mode() {
            return err;
        }
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

        // The creator becomes the owner. No caller identity means no auth
        // middleware is configured, so the channel is created with no owner
        // on record rather than failing the request.
        if let Some(c) = &caller {
            self.ownership
                .set_owner(channel_name.clone(), c.subject.clone())
                .await;
        }

        info!(caller = ?caller, "Created channel {channel_name}");
        self.success_response()
    }

    async fn handle_delete_channel(
        &self,
        req: DeleteChannelRequest,
        caller: Option<CallerIdentity>,
    ) -> CommandResponse {
        if let Some(err) = self.ensure_api_mode() {
            return err;
        }
        let channel_name = &req.channel_name;

        if let Err(e) = self.sessions.remove_session(channel_name, &self.app).await {
            error!("Failed to delete channel {channel_name}: {e}");
            return self.error_response(format!("{e}"));
        }
        self.ownership.remove_owner(channel_name).await;

        info!(caller = ?caller, "Deleted channel {channel_name}");
        self.success_response()
    }

    /// See `enforce_grant`.
    async fn check_grant(
        &self,
        channel_name: &str,
        participant_name: &str,
        caller: &Option<CallerIdentity>,
        grant: &Option<Vec<u8>>,
    ) -> Result<(), CommandResponse> {
        enforce_grant(
            &self.ownership,
            self.grant_verifier.as_ref(),
            channel_name,
            participant_name,
            caller,
            grant,
        )
        .await
        .map_err(|msg| self.error_response(msg))
    }

    async fn handle_add_participant(
        &self,
        req: AddParticipantRequest,
        caller: Option<CallerIdentity>,
    ) -> CommandResponse {
        if let Some(err) = self.ensure_api_mode() {
            return err;
        }
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self.error_response(format!("channel {channel_name} not found"));
            }
        };

        if let Err(resp) = self
            .check_grant(channel_name, participant_name_str, &caller, &req.grant)
            .await
        {
            return resp;
        }

        let participant_name = match ProtoName::parse_name(participant_name_str) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid participant name: {e}"));
            }
        };

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

        info!(
            caller = ?caller,
            "Added participant {participant_name_str} to channel {channel_name}"
        );
        self.success_response()
    }

    async fn handle_delete_participant(
        &self,
        req: DeleteParticipantRequest,
        caller: Option<CallerIdentity>,
    ) -> CommandResponse {
        if let Some(err) = self.ensure_api_mode() {
            return err;
        }
        let channel_name = &req.channel_name;
        let participant_name_str = &req.participant_name;

        let session = match self.sessions.get_session(channel_name).await {
            Some(s) => s,
            None => {
                return self.error_response(format!("channel {channel_name} not found"));
            }
        };

        if let Err(resp) = self
            .check_grant(channel_name, participant_name_str, &caller, &req.grant)
            .await
        {
            return resp;
        }

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

        info!(
            caller = ?caller,
            "Removed participant {participant_name_str} from channel {channel_name}"
        );
        self.success_response()
    }

    async fn handle_list_channels(&self, caller: Option<CallerIdentity>) -> ListChannelsResponse {
        let channel_names = self.sessions.list_channel_names().await;
        info!(caller = ?caller, "Listing channels, count: {}", channel_names.len());

        let mut channels = Vec::with_capacity(channel_names.len());
        for channel_name in &channel_names {
            channels.push(ChannelInfo {
                channel_name: channel_name.clone(),
                owner: self.ownership.get_owner(channel_name).await,
            });
        }

        ListChannelsResponse {
            success: true,
            error_msg: None,
            channel_name: channel_names,
            channels,
        }
    }

    async fn handle_list_participants(
        &self,
        req: ListParticipantsRequest,
        caller: Option<CallerIdentity>,
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

        let participant_names: Vec<String> = participants
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();

        info!(
            caller = ?caller,
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
        let caller = CallerIdentity::from_request(&request);
        let req = request.into_inner();
        debug!("Received create_channel request");
        Ok(Response::new(self.handle_create_channel(req, caller).await))
    }

    async fn delete_channel(
        &self,
        request: Request<DeleteChannelRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let caller = CallerIdentity::from_request(&request);
        let req = request.into_inner();
        debug!("Received delete_channel request");
        Ok(Response::new(self.handle_delete_channel(req, caller).await))
    }

    async fn add_participant(
        &self,
        request: Request<AddParticipantRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let caller = CallerIdentity::from_request(&request);
        let req = request.into_inner();
        debug!("Received add_participant request");
        Ok(Response::new(
            self.handle_add_participant(req, caller).await,
        ))
    }

    async fn delete_participant(
        &self,
        request: Request<DeleteParticipantRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let caller = CallerIdentity::from_request(&request);
        let req = request.into_inner();
        debug!("Received delete_participant request");
        Ok(Response::new(
            self.handle_delete_participant(req, caller).await,
        ))
    }

    async fn list_channels(
        &self,
        request: Request<ListChannelsRequest>,
    ) -> Result<Response<ListChannelsResponse>, Status> {
        let caller = CallerIdentity::from_request(&request);
        debug!("Received list_channels request");
        Ok(Response::new(self.handle_list_channels(caller).await))
    }

    async fn list_participants(
        &self,
        request: Request<ListParticipantsRequest>,
    ) -> Result<Response<ListParticipantsResponse>, Status> {
        let caller = CallerIdentity::from_request(&request);
        let req = request.into_inner();
        debug!("Received list_participants request");
        Ok(Response::new(
            self.handle_list_participants(req, caller).await,
        ))
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
            channels: vec![],
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
            channels: vec![],
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
            channels: vec![],
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
            grant: None,
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert_eq!(request.participant_name, "org/namespace/app");
    }

    #[test]
    fn test_delete_participant_request() {
        let request = DeleteParticipantRequest {
            channel_name: "org/namespace/channel".to_string(),
            participant_name: "org/namespace/app".to_string(),
            grant: None,
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

    // ── enforce_grant ─────────────────────────────────────────────────

    use crate::grant::DidKeyEd25519Verifier;
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

    /// Generates a fresh Ed25519 keypair and its did:key identifier.
    fn new_owner() -> (Ed25519KeyPair, String) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();

        let mut multicodec = vec![0xed, 0x01];
        multicodec.extend_from_slice(key_pair.public_key().as_ref());
        let did_key = format!("did:key:z{}", bs58::encode(multicodec).into_string());

        (key_pair, did_key)
    }

    fn sign_grant(
        key_pair: &Ed25519KeyPair,
        channel: &str,
        invitee: &str,
        not_after: u64,
    ) -> Vec<u8> {
        let payload = [channel, invitee, "member", &not_after.to_string(), "nonce"].join("\0");
        let signature = key_pair.sign(payload.as_bytes());
        serde_json::json!({
            "channel": channel,
            "invitee": invitee,
            "role": "member",
            "not_after": not_after,
            "nonce": "nonce",
            "signature": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, signature.as_ref()),
        })
        .to_string()
        .into_bytes()
    }

    const FAR_FUTURE: u64 = 9_999_999_999;

    #[tokio::test]
    async fn enforce_grant_allows_an_ownerless_channel_without_a_grant() {
        let ownership = ChannelOwnership::new();
        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn enforce_grant_allows_the_owner_without_a_grant() {
        let ownership = ChannelOwnership::new();
        let (_key, owner) = new_owner();
        ownership
            .set_owner("org/ns/ch1".to_string(), owner.clone())
            .await;
        let caller = Some(CallerIdentity { subject: owner });

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &caller,
            &None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn enforce_grant_denies_a_non_owner_with_no_grant() {
        let ownership = ChannelOwnership::new();
        let (_key, owner) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &None,
        )
        .await;
        assert!(result.unwrap_err().contains("requires a grant"));
    }

    #[tokio::test]
    async fn enforce_grant_allows_a_non_owner_with_a_valid_grant() {
        let ownership = ChannelOwnership::new();
        let (owner_key, owner) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;
        let grant = sign_grant(&owner_key, "org/ns/ch1", "org/ns/agent1", FAR_FUTURE);

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &Some(grant),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn enforce_grant_denies_a_grant_signed_by_someone_other_than_the_owner() {
        let ownership = ChannelOwnership::new();
        let (_, owner) = new_owner();
        let (impostor_key, _) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;
        let grant = sign_grant(&impostor_key, "org/ns/ch1", "org/ns/agent1", FAR_FUTURE);

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &Some(grant),
        )
        .await;
        assert!(result.unwrap_err().contains("invalid grant"));
    }

    #[tokio::test]
    async fn enforce_grant_denies_a_grant_for_a_different_channel() {
        let ownership = ChannelOwnership::new();
        let (owner_key, owner) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;
        let grant = sign_grant(
            &owner_key,
            "org/ns/other-channel",
            "org/ns/agent1",
            FAR_FUTURE,
        );

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &Some(grant),
        )
        .await;
        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn enforce_grant_denies_a_grant_for_a_different_invitee() {
        let ownership = ChannelOwnership::new();
        let (owner_key, owner) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;
        let grant = sign_grant(&owner_key, "org/ns/ch1", "org/ns/someone-else", FAR_FUTURE);

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &Some(grant),
        )
        .await;
        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn enforce_grant_denies_an_expired_grant() {
        let ownership = ChannelOwnership::new();
        let (owner_key, owner) = new_owner();
        ownership.set_owner("org/ns/ch1".to_string(), owner).await;
        let grant = sign_grant(&owner_key, "org/ns/ch1", "org/ns/agent1", 1);

        let result = enforce_grant(
            &ownership,
            &DidKeyEd25519Verifier,
            "org/ns/ch1",
            "org/ns/agent1",
            &None,
            &Some(grant),
        )
        .await;
        assert!(result.unwrap_err().contains("expired"));
    }
}
