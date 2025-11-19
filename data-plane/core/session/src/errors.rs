// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use thiserror::Error;

// Local crate
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::utils::MessageError;
use slim_auth::errors::AuthError;
use slim_mls::errors::MlsError;

#[derive(Error, Debug, PartialEq)]
pub enum SessionError {
    // Legacy generic String variants (scheduled for removal as call sites are migrated)
    #[error("error receiving message from slim instance: {0}")]
    SlimReception(String),
    #[error("error sending message to slim instance: {0}")]
    SlimTransmission(String),
    #[error("error in message forwarding: {0}")]
    Forward(String),
    #[error("error receiving message from app: {0}")]
    AppReception(String),
    #[error("error sending message to app: {0}")]
    AppTransmission(String),
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
    #[error("session not found: {0}")]
    SessionNotFound(u32),
    #[error("subscription not found: {0}")]
    SubscriptionNotFound(String),
    #[error("default for session not supported: {0}")]
    SessionDefaultNotSupported(String),
    #[error("missing session id: {0}")]
    MissingSessionId(String),
    #[error("error during message validation: {0}")]
    ValidationError(String),

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
    #[error("MLS encryption failed")]
    MlsEncryptionFailed,
    #[error("MLS decryption failed")]
    MlsDecryptionFailed,
    #[error("Encrypted message has no payload")]
    MlsNoPayload,

    // Identity
    #[error("identity error: {0}")]
    IdentityError(String),
    #[error("error pushing identity to the message")]
    IdentityPushError,

    // Auth / MLS typed propagation
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),
    #[error("mls operation error: {0}")]
    MlsOp(#[from] MlsError),

    // Handles
    #[error("no session handle available: session might be closed")]
    NoHandleAvailable,
    #[error("session error: {0}")]
    Generic(String),
    #[error("error receiving ack for message: {0}")]
    AckReception(String),

    // Structured (new) variants
    #[error("unexpected message type: {message_type:?}")]
    UnexpectedMessageType { message_type: ProtoSessionMessageType },

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

    #[error("missing MLS payload in {context} message")]
    MissingMlsPayload { context: &'static str },

    #[error("channel send failure: {source}")]
    ChannelSendFailure {
        #[from]
        source: tokio::sync::mpsc::error::SendError<Message>,
    },

    // Channel Endpoint (remaining legacy variants to be migrated)
    #[error("msl state is None")]
    NoMls,
    #[error("error generating key package: {0}")]
    MLSKeyPackage(String),
    #[error("invalid id message: {0}")]
    MLSIdMessage(String),
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

impl From<MessageError> for SessionError {
    fn from(err: MessageError) -> Self {
        // Prefer structured variants where possible; default to Processing for now.
        SessionError::Processing(err.to_string())
    }
}
