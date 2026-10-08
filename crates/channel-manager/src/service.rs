// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC service implementation for the Channel Manager.

use std::sync::Arc;
use std::time::Duration;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::{ProtoName, ProtoSessionType};
use slim_persistence::PersistenceError;
use slim_service::app::App;
use slim_session::completion_handle::CompletionHandle;
use slim_session::{SessionConfig, SessionError, session_config::MlsSettings};
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::approval::{ApprovalDecision, OwnerApprover, SlimOwnerApprover};
use crate::caller_identity::CallerIdentity;
use crate::expiry::ChannelExpiry;
use crate::grant::{DidKeyEd25519Verifier, GrantAction, GrantVerifier};
use crate::nonce::NonceStore;
use crate::ownership::{ChannelOwner, ChannelOwnership};
use crate::proto::channel_manager_service_server::ChannelManagerService;
use crate::proto::{
    AddParticipantRequest, ApprovalRequest, ChannelInfo, CommandResponse, CreateChannelRequest,
    DeleteChannelRequest, DeleteParticipantRequest, ListChannelsRequest, ListChannelsResponse,
    ListParticipantsRequest, ListParticipantsResponse, ParticipantAction,
};
use crate::sessions::SessionsList;
use crate::store::StateStore;

/// How long a channel owner gets to answer an approval request by default.
/// A human typically answers it, so this errs long.
const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Current time in unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A participant change being authorized.
struct ParticipantChange<'a> {
    channel: &'a str,
    participant: &'a str,
    action: GrantAction,
}

/// The approval protocol's name for a grant action.
fn approval_action(action: GrantAction) -> ParticipantAction {
    match action {
        GrantAction::Add => ParticipantAction::Add,
        GrantAction::Delete => ParticipantAction::Delete,
    }
}

/// What authorizing a participant change needs from the server -- borrowed,
/// so the policy is testable without a full `ChannelManagerServer` (which
/// needs a real `App`).
struct GrantPolicy<'a> {
    ownership: &'a ChannelOwnership,
    nonces: &'a NonceStore,
    verifier: &'a dyn GrantVerifier,
    approver: &'a dyn OwnerApprover,
    approval_timeout: Duration,
}

impl GrantPolicy<'_> {
    /// Enforces the grant contract for a mutating participant-change RPC.
    ///
    /// A channel with no owner on record needs no grant -- unchanged,
    /// pre-ownership behavior. The owner needs no grant for their own
    /// channel. Anyone else needs one: presented on the request, or -- when
    /// none is and the owner left a callback name -- obtained by asking the
    /// owner, with no answer within `approval_timeout` counting as a denial.
    /// Either way, the grant must verify, name this exact channel,
    /// participant and action, not be expired, and not have been used
    /// before.
    async fn enforce(
        &self,
        change: &ParticipantChange<'_>,
        caller: &Option<CallerIdentity>,
        grant: &Option<Vec<u8>>,
    ) -> Result<(), String> {
        let Some(owner) = self.ownership.get_owner(change.channel).await else {
            return Ok(());
        };
        if caller.as_ref().is_some_and(|c| c.subject == owner.subject) {
            return Ok(());
        }

        match grant {
            Some(grant) => self.verify(&owner.subject, change, grant),
            None => {
                let grant = self.ask_owner(&owner, change, caller).await?;
                self.verify(&owner.subject, change, &grant)
            }
        }
    }

    async fn ask_owner(
        &self,
        owner: &ChannelOwner,
        change: &ParticipantChange<'_>,
        caller: &Option<CallerIdentity>,
    ) -> Result<Vec<u8>, String> {
        let Some(callback_name) = &owner.callback_name else {
            return Err(format!(
                "channel {} requires a grant from its owner",
                change.channel
            ));
        };
        let request = ApprovalRequest {
            channel_name: change.channel.to_string(),
            participant_name: change.participant.to_string(),
            action: approval_action(change.action) as i32,
            requester: caller.as_ref().map(|c| c.subject.clone()),
        };

        let answer = tokio::time::timeout(
            self.approval_timeout,
            self.approver.request_approval(callback_name, request),
        )
        .await;
        match answer {
            Err(_) => Err("channel owner did not answer the approval request in time".to_string()),
            Ok(Err(e)) => Err(format!("could not ask the channel owner for approval: {e}")),
            Ok(Ok(ApprovalDecision::Denied(reason))) => {
                Err(format!("channel owner denied the request: {reason}"))
            }
            Ok(Ok(ApprovalDecision::Granted(grant))) => Ok(grant),
        }
    }

    fn verify(
        &self,
        owner_subject: &str,
        change: &ParticipantChange<'_>,
        grant: &[u8],
    ) -> Result<(), String> {
        let parsed = self
            .verifier
            .verify(owner_subject, grant)
            .map_err(|e| format!("invalid grant: {e}"))?;

        if parsed.channel != change.channel
            || parsed.invitee != change.participant
            || parsed.action != change.action
        {
            return Err("grant does not authorize this change".to_string());
        }

        let now = unix_now();
        if parsed.not_after < now {
            return Err("grant has expired".to_string());
        }

        // Last, so a grant rejected for any other reason isn't burned. If
        // its use can't be recorded durably, refuse it rather than accept a
        // grant that would be replayable after a restart.
        let first_use = self
            .nonces
            .consume(&parsed.channel, &parsed.nonce, parsed.not_after, now)
            .map_err(|e| format!("could not record the grant's use: {e}"))?;
        if !first_use {
            return Err("grant has already been used".to_string());
        }

        Ok(())
    }
}

/// gRPC server for the Channel Manager service
pub struct ChannelManagerServer {
    app: Arc<App<AuthProvider, AuthVerifier>>,
    sessions: Arc<SessionsList>,
    ownership: ChannelOwnership,
    expiry: ChannelExpiry,
    nonces: NonceStore,
    grant_verifier: Arc<dyn GrantVerifier>,
    owner_approver: Arc<dyn OwnerApprover>,
    approval_timeout: Duration,
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
    /// Grants presented by non-owners are verified with
    /// [`DidKeyEd25519Verifier`]; use [`Self::with_grant_verifier`] to swap in
    /// a different grant format or key scheme. Owners are asked for approval
    /// over SLIM with [`SlimOwnerApprover`] (on `conn_id`), given
    /// [`DEFAULT_APPROVAL_TIMEOUT`] to answer; see
    /// [`Self::with_owner_approver`] and [`Self::with_approval_timeout`].
    pub fn new(
        app: Arc<App<AuthProvider, AuthVerifier>>,
        conn_id: u64,
        sessions: Arc<SessionsList>,
        config_mode: bool,
    ) -> Self {
        if let Err(e) = app.set_default_gateway(conn_id) {
            warn!("failed to pin default gateway to conn {conn_id}: {e}");
        }

        Self {
            owner_approver: Arc::new(SlimOwnerApprover::new(app.clone(), Some(conn_id))),
            app,
            sessions,
            ownership: ChannelOwnership::new(),
            expiry: ChannelExpiry::new(),
            nonces: NonceStore::new(),
            grant_verifier: Arc::new(DidKeyEd25519Verifier),
            approval_timeout: DEFAULT_APPROVAL_TIMEOUT,
            config_mode,
        }
    }

    /// Replaces the default grant verifier (see `crate::grant::GrantVerifier`).
    pub fn with_grant_verifier(mut self, grant_verifier: Arc<dyn GrantVerifier>) -> Self {
        self.grant_verifier = grant_verifier;
        self
    }

    /// Replaces the default owner approver (see
    /// `crate::approval::OwnerApprover`).
    pub fn with_owner_approver(mut self, owner_approver: Arc<dyn OwnerApprover>) -> Self {
        self.owner_approver = owner_approver;
        self
    }

    /// Sets how long a channel owner gets to answer an approval request
    /// before it counts as a denial.
    pub fn with_approval_timeout(mut self, approval_timeout: Duration) -> Self {
        self.approval_timeout = approval_timeout;
        self
    }

    /// Persists channel ownership, expiry and consumed grant nonces in
    /// `store`, so they survive a restart along with the sessions they
    /// govern. Without it, a channel restored from session persistence would
    /// come back with no owner -- and so with no grant required to change
    /// it -- and no expiry.
    ///
    /// Call after the sessions list has been restored: owners and expiry are
    /// kept only for channels it contains, and dropped (from the store too)
    /// for the rest.
    pub async fn with_state_store(mut self, store: StateStore) -> Result<Self, PersistenceError> {
        let restored: std::collections::HashSet<String> = self
            .sessions
            .list_channel_names()
            .await
            .into_iter()
            .collect();
        self.ownership = ChannelOwnership::load(store.clone(), |c| restored.contains(c))?;
        self.expiry = ChannelExpiry::load(store.clone(), |c| restored.contains(c))?;
        self.nonces = NonceStore::load(store, unix_now())?;
        Ok(self)
    }

    /// Spawns the task that deletes channels whose TTL has passed, checking
    /// every `interval`. The first check runs immediately, deleting channels
    /// that expired while the channel manager was down. Abort the returned
    /// handle on shutdown.
    pub fn spawn_reaper(self: &Arc<Self>, interval: Duration) -> JoinHandle<()> {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                server.reap_expired(unix_now()).await;
            }
        })
    }

    async fn reap_expired(&self, now: u64) {
        for channel in self.expiry.expired(now).await {
            match self.sessions.remove_session(&channel, &self.app).await {
                Ok(()) => info!(%channel, "Deleted expired channel"),
                Err(e) => warn!(%channel, error = %e, "Failed to delete expired channel"),
            }
            // remove_session drops the channel from the list even when
            // deleting its SLIM session fails, so there's nothing to retry:
            // forget its state either way, or this would warn about it on
            // every pass.
            self.forget_channel(&channel).await;
        }
    }

    /// Drops a deleted channel's owner and expiry records. Store errors are
    /// only logged: the stale records are dropped at the next startup, since
    /// the channel won't be restored.
    async fn forget_channel(&self, channel_name: &str) {
        if let Err(e) = self.ownership.remove_owner(channel_name).await {
            warn!("Failed to delete owner record of channel {channel_name}: {e}");
        }
        if let Err(e) = self.expiry.remove_expiry(channel_name).await {
            warn!("Failed to delete expiry record of channel {channel_name}: {e}");
        }
    }

    /// Records a new channel's expiry and owner, if any -- durably, when
    /// there's a store.
    async fn record_new_channel(
        &self,
        channel_name: &str,
        expires_at: Option<u64>,
        owner: Option<ChannelOwner>,
    ) -> Result<(), PersistenceError> {
        if let Some(expires_at) = expires_at {
            self.expiry
                .set_expiry(channel_name.to_string(), expires_at)
                .await?;
        }
        if let Some(owner) = owner {
            self.ownership
                .set_owner(channel_name.to_string(), owner)
                .await?;
        }
        Ok(())
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

        if req.ttl_seconds == Some(0) {
            return self.error_response("ttl-seconds must be positive".to_string());
        }

        // Parse the channel name
        let name = match ProtoName::parse_name(channel_name) {
            Ok(n) => n,
            Err(e) => {
                return self.error_response(format!("invalid channel name: {e}"));
            }
        };

        // Reject a malformed callback name now rather than at the first
        // approval request, when the owner is no longer around to fix it.
        if let Some(callback_name) = &req.owner_callback_name
            && let Err(e) = ProtoName::parse_name(callback_name)
        {
            return self.error_response(format!("invalid owner callback name: {e}"));
        }

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
        let owner = caller.as_ref().map(|c| ChannelOwner {
            subject: c.subject.clone(),
            callback_name: req.owner_callback_name.clone(),
        });
        let expires_at = req.ttl_seconds.map(|ttl| unix_now().saturating_add(ttl));
        if let Err(e) = self
            .record_new_channel(channel_name, expires_at, owner)
            .await
        {
            // Not recorded durably, the channel would come back from a
            // restart ownerless -- and so ungated -- or never expiring: undo
            // it.
            error!("Failed to record state of channel {channel_name}: {e}");
            if let Err(e) = self.sessions.remove_session(channel_name, &self.app).await {
                error!("Failed to roll back channel {channel_name}: {e}");
            }
            self.forget_channel(channel_name).await;
            return self
                .error_response(format!("failed to record state of channel {channel_name}"));
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
        self.forget_channel(channel_name).await;

        info!(caller = ?caller, "Deleted channel {channel_name}");
        self.success_response()
    }

    /// See `GrantPolicy::enforce`.
    async fn check_grant(
        &self,
        change: &ParticipantChange<'_>,
        caller: &Option<CallerIdentity>,
        grant: &Option<Vec<u8>>,
    ) -> Result<(), CommandResponse> {
        let policy = GrantPolicy {
            ownership: &self.ownership,
            nonces: &self.nonces,
            verifier: self.grant_verifier.as_ref(),
            approver: self.owner_approver.as_ref(),
            approval_timeout: self.approval_timeout,
        };
        policy
            .enforce(change, caller, grant)
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
            .check_grant(
                &ParticipantChange {
                    channel: channel_name,
                    participant: participant_name_str,
                    action: GrantAction::Add,
                },
                &caller,
                &req.grant,
            )
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
            .check_grant(
                &ParticipantChange {
                    channel: channel_name,
                    participant: participant_name_str,
                    action: GrantAction::Delete,
                },
                &caller,
                &req.grant,
            )
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
                owner: self
                    .ownership
                    .get_owner(channel_name)
                    .await
                    .map(|o| o.subject),
                expires_at: self.expiry.get_expiry(channel_name).await,
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
            owner_callback_name: None,
            ttl_seconds: None,
        };
        assert_eq!(request.channel_name, "org/namespace/channel");
        assert!(request.mls_enabled);
    }

    #[test]
    fn test_create_channel_request_mls_disabled() {
        let request = CreateChannelRequest {
            channel_name: "org/namespace/channel".to_string(),
            mls_enabled: false,
            owner_callback_name: None,
            ttl_seconds: None,
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

    // ── GrantPolicy ───────────────────────────────────────────────────

    use crate::approval::ApprovalError;
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

    /// Signs an "add" grant.
    fn sign_grant(
        key_pair: &Ed25519KeyPair,
        channel: &str,
        invitee: &str,
        not_after: u64,
    ) -> Vec<u8> {
        sign_grant_for(key_pair, channel, invitee, "add", not_after)
    }

    /// Builds the default verifier's wire format by hand, independently of
    /// `crate::grant`'s own helpers, so these tests also pin that format.
    fn sign_grant_for(
        key_pair: &Ed25519KeyPair,
        channel: &str,
        invitee: &str,
        action: &str,
        not_after: u64,
    ) -> Vec<u8> {
        let payload = [
            channel,
            invitee,
            action,
            "member",
            &not_after.to_string(),
            "nonce",
        ]
        .join("\0");
        let signature = key_pair.sign(payload.as_bytes());
        serde_json::json!({
            "channel": channel,
            "invitee": invitee,
            "action": action,
            "role": "member",
            "not_after": not_after,
            "nonce": "nonce",
            "signature": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, signature.as_ref()),
        })
        .to_string()
        .into_bytes()
    }

    const FAR_FUTURE: u64 = 9_999_999_999;
    const CHANNEL: &str = "org/ns/ch1";
    const AGENT: &str = "org/ns/agent1";
    const CALLBACK: &str = "org/ns/owner-shadi";

    /// Answers every approval request with `answer` -- or never, if `None` --
    /// recording each (callback name, request) it was asked.
    struct FakeApprover {
        answer: Option<Result<ApprovalDecision, ApprovalError>>,
        received: parking_lot::Mutex<Vec<(String, ApprovalRequest)>>,
    }

    impl FakeApprover {
        fn answering(answer: Result<ApprovalDecision, ApprovalError>) -> Self {
            Self {
                answer: Some(answer),
                received: Default::default(),
            }
        }

        fn silent() -> Self {
            Self {
                answer: None,
                received: Default::default(),
            }
        }

        fn calls(&self) -> Vec<(String, ApprovalRequest)> {
            self.received.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl OwnerApprover for FakeApprover {
        async fn request_approval(
            &self,
            callback_name: &str,
            request: ApprovalRequest,
        ) -> Result<ApprovalDecision, ApprovalError> {
            self.received
                .lock()
                .push((callback_name.to_string(), request));
            match &self.answer {
                Some(answer) => answer.clone(),
                None => std::future::pending().await,
            }
        }
    }

    fn policy<'a>(
        ownership: &'a ChannelOwnership,
        nonces: &'a NonceStore,
        approver: &'a FakeApprover,
    ) -> GrantPolicy<'a> {
        GrantPolicy {
            ownership,
            nonces,
            verifier: &DidKeyEd25519Verifier,
            approver,
            approval_timeout: Duration::from_millis(100),
        }
    }

    fn add() -> ParticipantChange<'static> {
        ParticipantChange {
            channel: CHANNEL,
            participant: AGENT,
            action: GrantAction::Add,
        }
    }

    /// A channel owned by a fresh key, optionally reachable at `CALLBACK`.
    async fn owned_channel(callback: bool) -> (ChannelOwnership, Ed25519KeyPair, String) {
        let ownership = ChannelOwnership::new();
        let (key, subject) = new_owner();
        ownership
            .set_owner(
                CHANNEL.to_string(),
                ChannelOwner {
                    subject: subject.clone(),
                    callback_name: callback.then(|| CALLBACK.to_string()),
                },
            )
            .await
            .unwrap();
        (ownership, key, subject)
    }

    #[tokio::test]
    async fn allows_an_ownerless_channel_without_a_grant() {
        let nonces = NonceStore::new();
        let ownership = ChannelOwnership::new();
        let approver = FakeApprover::silent();
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.is_ok());
        assert!(approver.calls().is_empty());
    }

    #[tokio::test]
    async fn allows_the_owner_without_a_grant() {
        let nonces = NonceStore::new();
        let (ownership, _key, subject) = owned_channel(true).await;
        let approver = FakeApprover::silent();
        let caller = Some(CallerIdentity { subject });
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &caller, &None)
            .await;
        assert!(result.is_ok());
        assert!(approver.calls().is_empty());
    }

    #[tokio::test]
    async fn denies_a_non_owner_with_no_grant_and_no_callback() {
        let nonces = NonceStore::new();
        let (ownership, _key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.unwrap_err().contains("requires a grant"));
        assert!(approver.calls().is_empty());
    }

    #[tokio::test]
    async fn allows_a_non_owner_with_a_valid_presented_grant() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn a_presented_grant_is_used_without_asking_the_owner() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.is_ok());
        assert!(approver.calls().is_empty());
    }

    #[tokio::test]
    async fn denies_a_presented_grant_signed_by_someone_other_than_the_owner() {
        let nonces = NonceStore::new();
        let (ownership, _, _) = owned_channel(false).await;
        let (impostor, _) = new_owner();
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&impostor, CHANNEL, AGENT, FAR_FUTURE));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.unwrap_err().contains("invalid grant"));
    }

    #[tokio::test]
    async fn denies_a_presented_grant_for_a_different_channel() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, "org/ns/other-channel", AGENT, FAR_FUTURE));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn denies_a_presented_grant_for_a_different_invitee() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, CHANNEL, "org/ns/someone-else", FAR_FUTURE));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn denies_an_expired_presented_grant() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, 1));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &grant)
            .await;
        assert!(result.unwrap_err().contains("expired"));
    }

    #[tokio::test]
    async fn asks_the_owner_and_allows_with_the_grant_they_return() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(sign_grant(
            &key, CHANNEL, AGENT, FAR_FUTURE,
        ))));
        let caller = Some(CallerIdentity {
            subject: "did:key:z6Mkrequester".to_string(),
        });

        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &caller, &None)
            .await;

        assert!(result.is_ok());
        assert_eq!(
            approver.calls(),
            vec![(
                CALLBACK.to_string(),
                ApprovalRequest {
                    channel_name: CHANNEL.to_string(),
                    participant_name: AGENT.to_string(),
                    action: ParticipantAction::Add as i32,
                    requester: Some("did:key:z6Mkrequester".to_string()),
                }
            )]
        );
    }

    #[tokio::test]
    async fn tells_the_owner_which_action_is_being_requested() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(sign_grant_for(
            &key, CHANNEL, AGENT, "delete", FAR_FUTURE,
        ))));
        let delete = ParticipantChange {
            action: GrantAction::Delete,
            ..add()
        };

        let result = policy(&ownership, &nonces, &approver)
            .enforce(&delete, &None, &None)
            .await;

        assert!(result.is_ok());
        assert_eq!(
            approver.calls()[0].1.action,
            ParticipantAction::Delete as i32
        );
    }

    #[tokio::test]
    async fn denies_an_add_grant_presented_for_a_delete() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE));
        let delete = ParticipantChange {
            action: GrantAction::Delete,
            ..add()
        };

        let result = policy(&ownership, &nonces, &approver)
            .enforce(&delete, &None, &grant)
            .await;

        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn denies_a_returned_grant_for_a_different_action() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        // Asked to approve a delete, the owner returns a grant to add.
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(sign_grant(
            &key, CHANNEL, AGENT, FAR_FUTURE,
        ))));
        let delete = ParticipantChange {
            action: GrantAction::Delete,
            ..add()
        };

        let result = policy(&ownership, &nonces, &approver)
            .enforce(&delete, &None, &None)
            .await;

        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn denies_when_the_owner_declines() {
        let nonces = NonceStore::new();
        let (ownership, _, _) = owned_channel(true).await;
        let approver =
            FakeApprover::answering(Ok(ApprovalDecision::Denied("not this agent".to_string())));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        let err = result.unwrap_err();
        assert!(err.contains("denied the request"));
        assert!(err.contains("not this agent"));
    }

    #[tokio::test]
    async fn denies_when_the_owner_does_not_answer_in_time() {
        let nonces = NonceStore::new();
        let (ownership, _, _) = owned_channel(true).await;
        let approver = FakeApprover::silent();
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.unwrap_err().contains("in time"));
        assert_eq!(approver.calls().len(), 1);
    }

    #[tokio::test]
    async fn denies_when_the_owner_cannot_be_reached() {
        let nonces = NonceStore::new();
        let (ownership, _, _) = owned_channel(true).await;
        let approver =
            FakeApprover::answering(Err(ApprovalError::Transport("no route".to_string())));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.unwrap_err().contains("could not ask"));
    }

    #[tokio::test]
    async fn verifies_a_grant_returned_by_the_owner_like_a_presented_one() {
        let nonces = NonceStore::new();
        let (ownership, _, _) = owned_channel(true).await;
        let (impostor, _) = new_owner();
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(sign_grant(
            &impostor, CHANNEL, AGENT, FAR_FUTURE,
        ))));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.unwrap_err().contains("invalid grant"));
    }

    #[tokio::test]
    async fn denies_a_returned_grant_for_a_different_invitee() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(sign_grant(
            &key,
            CHANNEL,
            "org/ns/someone-else",
            FAR_FUTURE,
        ))));
        let result = policy(&ownership, &nonces, &approver)
            .enforce(&add(), &None, &None)
            .await;
        assert!(result.unwrap_err().contains("does not authorize"));
    }

    #[tokio::test]
    async fn refuses_a_presented_grant_used_twice() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let policy = policy(&ownership, &nonces, &approver);
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE));

        assert!(policy.enforce(&add(), &None, &grant).await.is_ok());
        let replay = policy.enforce(&add(), &None, &grant).await;
        assert!(replay.unwrap_err().contains("already been used"));
    }

    #[tokio::test]
    async fn refuses_a_grant_the_owner_returned_when_replayed() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(true).await;
        let grant = sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE);
        let approver = FakeApprover::answering(Ok(ApprovalDecision::Granted(grant.clone())));
        let policy = policy(&ownership, &nonces, &approver);

        assert!(policy.enforce(&add(), &None, &None).await.is_ok());
        let replay = policy.enforce(&add(), &None, &Some(grant)).await;
        assert!(replay.unwrap_err().contains("already been used"));
    }

    #[tokio::test]
    async fn a_grant_rejected_for_another_reason_is_not_burned() {
        let nonces = NonceStore::new();
        let (ownership, key, _) = owned_channel(false).await;
        let approver = FakeApprover::silent();
        let policy = policy(&ownership, &nonces, &approver);
        let grant = Some(sign_grant(&key, CHANNEL, AGENT, FAR_FUTURE));
        let someone_else = ParticipantChange {
            participant: "org/ns/someone-else",
            ..add()
        };

        let wrong_invitee = policy.enforce(&someone_else, &None, &grant).await;
        assert!(wrong_invitee.unwrap_err().contains("does not authorize"));
        assert!(policy.enforce(&add(), &None, &grant).await.is_ok());
    }
}
