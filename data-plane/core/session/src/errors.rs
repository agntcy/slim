// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use thiserror::Error;

// Local crate
use slim_auth::errors::AuthError;
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType};
use slim_datapath::messages::utils::MessageError;
use slim_mls::errors::MlsError;

#[derive(Error, Debug)]
pub enum SessionError {
    // Legacy generic String variants (scheduled for removal as call sites are migrated)
    #[error("error receiving message from slim instance: {0}")]
    SlimReception(String),
    // Removed legacy SlimTransmission / AppTransmission; use ChannelSendFailure or AppChannelSendFailure
    #[error("error in message forwarding: {0}")]
    Forward(String),
    #[error("error receiving message from app: {0}")]
    AppReception(String),
    #[error("error processing message: {0}")]
    Processing(String),
    #[error("error sending message to session: {0}")]
    QueueFullError(String),
    #[error("session id already used: {0}")]
    SessionIdAlreadyUsed(String),
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("missing SLIM header: {0}")]
    MissingSlimHeader(String),
    #[error("missing session header")]
    MissingSessionHeader,
    #[error("missing channel name")]
    MissingChannelName,
    #[error("session unknown: {0}")]
    SessionUnknown(String),
    #[error("subscription not found: {0}")]
    SubscriptionNotFound(String),
    #[error("default for session not supported: {0}")]
    SessionDefaultNotSupported(String),
    #[error("missing session id: {0}")]
    MissingSessionId(String),
    #[error("error during message validation: {0}")]
    ValidationError(String),

    // Not found errors
    #[error("session not found")]
    SessionNotFound,
    #[error("session not found: {id}")]
    SessionNotFoundWithId { id: u32 },

    // Timeouts
    #[error("message={message_id} session={session_id}: timeout")]
    Timeout {
        session_id: u32,
        message_id: u32,
        message: Box<Message>,
    },
    #[error("close session: operation timed out")]
    CloseTimeout,

    // Configuration / lifecycle
    #[error("configuration error: {0}")]
    ConfigurationError(String),
    #[error("message lost: {0}")]
    MessageLost(String),
    #[error("session closed")]
    SessionClosed, // replaced String variant
    #[error("interceptor error: {0}")]
    InterceptorError(String),

    // MLS encryption/decryption
    #[error("mls operation error: {0}")]
    MlsOp(#[from] MlsError),

    // Identity
    #[error("identity error: {0}")]
    IdentityError(String),
    #[error("error pushing identity to the message")]
    IdentityPushError,

    // Auth / MLS typed propagation
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    // Handles
    #[error("no session handle available: session might be closed")]
    NoHandleAvailable,
    #[error("session error: {0}")]
    Generic(String),
    #[error("error receiving ack for message: {0}")]
    AckReception(String),

    // Structured (new) variants
    #[error("unexpected message type: {message_type:?}")]
    UnexpectedMessageType {
        message_type: ProtoSessionMessageType,
    },

    #[error("missing payload: {context}")]
    MissingPayload { context: &'static str },

    #[error("participant not found")]
    ParticipantNotFound,

    #[error("cannot invite participant to point-to-point session")]
    CannotInviteToP2P,
    #[error("cannot remove participant from point-to-point session")]
    CannotRemoveFromP2P,
    #[error("only initiator can modify participants")]
    NotInitiator,

    #[error("invalid join request payload")]
    InvalidJoinRequestPayload,

    #[error("session is draining; cannot accept new messages")]
    DrainingBlocked,
    #[error("sender closed; drop message")]
    SenderClosed,

    #[error("missing new participant in GroupAdd message")]
    MissingNewParticipant,
    #[error("missing removed participant in GroupRemove message")]
    MissingRemovedParticipant,

    #[error("channel send failure")]
    ChannelSend,
    #[error("channel closed")]
    ChannelClosed,

    // Structured drain / closed states replacing generic Processing(String) usages
    #[error("sender closed; drop message")]
    SenderClosedDrop,
    #[error("receiver closed; drop message")]
    ReceiverClosedDrop,
    #[error("drain initiated; reject new messages")]
    DrainStartedRejectNew,
    #[error("session already closed")]
    SessionAlreadyClosed,
    #[error("session cleanup failed: {details}")]
    SessionCleanupFailed { details: String },
    #[error("message send retries exhausted for id={id}")]
    MessageSendRetryFailed { id: u32 },
    #[error("message receive retries exhausted for id={id}")]
    MessageReceiveRetryFailed { id: u32 },

    // Typed propagation of slim_datapath MessageError instead of stringly Processing(...)
    #[error("datapath message error: {0}")]
    DatapathMessage(#[from] MessageError),

    // More specific extraction/build contexts (optional use at call sites)
    #[error("message build failed: {0}")]
    MessageBuild(MessageError),
    #[error("message payload extract failed in {context}: {source}")]
    PayloadExtract {
        context: &'static str,
        source: MessageError,
    },

    // Channel Endpoint (remaining legacy variants to be migrated)
    #[error("error processing welcome message: {0}")]
    WelcomeMessage(String),
    #[error("error processing proposal message: {0}")]
    ParseProposalMessage(String),
    #[error("error creating proposal message: {0}")]
    NewProposalMessage(String),
    #[error("no pending requests for the given key: {0}")]
    TimerNotFound(String),
    #[error("error processing payload of Join Channel request: {0}")]
    JoinChannelPayload(String),
    #[error("key rotation pending")]
    KeyRotationPending,

    // Moderator Tasks errors
    #[error("error updating a task: {0}")]
    ModeratorTask(String),
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
    pub fn already_closed() -> Self {
        SessionError::SessionAlreadyClosed
    }

    // Helpers to construct new structured retry failure variants
    pub fn send_retry_failed(id: u32) -> Self {
        SessionError::MessageSendRetryFailed { id }
    }

    pub fn receive_retry_failed(id: u32) -> Self {
        SessionError::MessageReceiveRetryFailed { id }
    }
}

// Generic conversion from tokio mpsc SendError into a structured variant.
// The payload type is discarded; callers can still log details if needed.
impl<T> From<tokio::sync::mpsc::error::SendError<T>> for SessionError {
    fn from(_e: tokio::sync::mpsc::error::SendError<T>) -> Self {
        SessionError::ChannelSend
    }
}
