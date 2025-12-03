// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::messages::Name;
// Third-party crates
use thiserror::Error;

// Local crate
use slim_auth::errors::AuthError;
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::utils::MessageError;
use slim_mls::errors::MlsError;
use tonic::Status;

use crate::SessionMessage;

#[derive(Error, Debug)]
pub enum SessionError {
    // Transport and channel errors
    #[error("SLIM channel closed")]
    SlimChannelClosed,
    #[error("error receiving message from SLIM: {0}")]
    SlimReception(#[from] Status),

    // Message processing and validation errors
    #[error("message error: {0}")]
    MessageError(#[from] MessageError),
    #[error("missing removed participant in GroupRemove message")]
    MissingRemovedParticipantInGroupRemove,
    #[error("ping state not initialized")]
    PingStateNotInitialized,
    #[error("missing channel name for group session")]
    MissingChannelName,
    #[error("session type unknown: {0:?}")]
    SessionTypeUnknown(ProtoSessionType),
    #[error("session message type unexpected: {0:?}")]
    SessionMessageInternalUnexpected(Box<SessionMessage>),
    #[error("session message type unknown: {0:?}")]
    SessionMessageTypeUnknown(ProtoSessionMessageType),
    #[error("session message type unexpected: {0:?}")]
    SessionMessageTypeUnexpected(ProtoSessionMessageType),

    // Lookup and missing entities
    #[error("session not found: {0}")]
    SessionNotFound(u32),
    #[error("subscription not found: {0}")]
    SubscriptionNotFound(Name),

    // Session lifecycle and state
    #[error("session builder: not all required fields set")]
    SessionBuilderIncomplete,
    #[error("message lost for session id: {0}")]
    MessageLost(u32),
    #[error("session closed")]
    SessionClosed,
    #[error("session id already used: {0}")]
    SessionIdAlreadyUsed(u32),
    #[error("invalid session id: {0}")]
    InvalidSessionId(u32),

    // Cryptography (MLS)
    #[error("mls operation error: {0}")]
    MlsOp(#[from] MlsError),

    // Authorization and roles
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    // Acknowledgements and routing
    #[error("error receiving ack for message: {0}")]
    AckReception(String),
    #[error("unknown destination: {0}")]
    UnknownDestination(Name),

    // Session membership and permissions
    #[error("participant not found in group: {0}")]
    ParticipantNotFound(Name),
    #[error("cannot invite participant to point-to-point session")]
    CannotInviteToP2P,
    #[error("cannot remove participant from point-to-point session")]
    CannotRemoveFromP2P,
    #[error("only initiator can modify participants")]
    NotInitiator,

    // Routing and delivery failures
    #[error("error sending session internal message to session controller")]
    SessionControllerSendFailed,
    #[error("error sending new session notification to app")]
    NewSessionSendFailed,
    #[error("error sending session delete message to session layer")]
    SessionDeleteMessageSendFailed,
    #[error("error sending data message to application")]
    ApplicationMessageSendFailed,
    #[error("error sending data message to slim")]
    SlimMessageSendFailed,

    // Session lifecycle and state (continued)
    #[error("session is draining - drop message")]
    SessionDrainingDrop,
    #[error("session already closed")]
    SessionAlreadyClosed,
    #[error("session cleanup failed: {details}")]
    SessionCleanupFailed { details: String },
    #[error("message send retries exhausted for id={id}")]
    MessageSendRetryFailed { id: u32 },
    #[error("message receive retries exhausted for id={id}")]
    MessageReceiveRetryFailed { id: u32 },

    // Message construction and extraction contexts
    #[error("missing payload: {context}")]
    MissingPayload { context: &'static str },
    #[error("message build failed: {0}")]
    MessageBuild(MessageError),
    #[error("message payload extract failed in {context}: {source}")]
    PayloadExtract {
        context: &'static str,
        source: MessageError,
    },

    // Participant connectivity
    #[error("missing mls payload in welcome message")]
    WelcomeMessageMissingMlsPayload,
    #[error("invalid join request payload")]
    InvalidJoinRequestPayload,
    #[error("participant disconnected: {0}")]
    ParticipantDisconnected(Name),

    // Moderator task orchestration
    #[error("no pending requests for the given key: {0}")]
    TimerNotFound(u32),
    #[error("phase not supported for task")]
    ModeratorTaskUnsupportedPhase,
    #[error("unexpected timer id: {0}")]
    ModeratorTaskUnexpectedTimerId(u32),
    #[error("failed to add participant to session")]
    ModeratorTaskAddFailed,
    #[error("failed to remove participant from session")]
    ModeratorTaskRemoveFailed,
    #[error("failed to update session")]
    ModeratorTaskUpdateFailed,
    #[error("failed to close session")]
    ModeratorTaskCloseFailed,
}

impl SessionError {
    // Helper constructors for structured mapping without repeating strings.
    pub fn build_error(err: MessageError) -> Self {
        SessionError::MessageBuild(err)
    }
    pub fn extract_error(context: &'static str, err: MessageError) -> Self {
        SessionError::PayloadExtract {
            context,
            source: err,
        }
    }
    pub fn cleanup_failed<E: std::fmt::Display>(e: E) -> Self {
        SessionError::SessionCleanupFailed {
            details: e.to_string(),
        }
    }

    // Helpers to construct new structured retry failure variants
    pub fn send_retry_failed(id: u32) -> Self {
        SessionError::MessageSendRetryFailed { id }
    }

    pub fn receive_retry_failed(id: u32) -> Self {
        SessionError::MessageReceiveRetryFailed { id }
    }
}
