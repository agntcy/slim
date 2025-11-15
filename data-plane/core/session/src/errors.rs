// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use thiserror::Error;

// Local crate
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::utils::MessageError;

#[derive(Error, Debug, PartialEq)]
pub enum SessionError {
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
    #[error("message={message_id} session={session_id}: timeout")]
    Timeout {
        session_id: u32,
        message_id: u32,
        message: Box<Message>,
    },
    #[error("close session: operation timed out")]
    CloseTimeout,
    #[error("configuration error: {0}")]
    ConfigurationError(String),
    #[error("message lost: {0}")]
    MessageLost(String),
    #[error("session closed: {0}")]
    SessionClosed(String),
    #[error("interceptor error: {0}")]
    InterceptorError(String),
    #[error("MLS encryption failed: {0}")]
    MlsEncryptionFailed(String),
    #[error("MLS decryption failed: {0}")]
    MlsDecryptionFailed(String),
    #[error("Encrypted message has no payload")]
    MlsNoPayload,
    #[error("identity error: {0}")]
    IdentityError(String),
    #[error("error pushing identity to the message: {0}")]
    IdentityPushError(String),
    #[error("no session handle available: session might be closed")]
    NoHandleAvailable,
    #[error("session error: {0}")]
    Generic(String),
    #[error("error receiving ack for message: {0}")]
    AckReception(String),

    // Channel Endpoint errors
    #[error("error initializing MLS: {0}")]
    MLSInit(String),
    #[error("msl state is None")]
    NoMls,
    #[error("error generating key package: {0}")]
    MLSKeyPackage(String),
    #[error("invalid id message: {0}")]
    MLSIdMessage(String),
    #[error("error processing welcome message: {0}")]
    WelcomeMessage(String),
    #[error("error processing commit message: {0}")]
    CommitMessage(String),
    #[error("error processing proposal message: {0}")]
    ParseProposalMessage(String),
    #[error("error creating proposal message: {0}")]
    NewProposalMessage(String),
    #[error("error adding a new participant: {0}")]
    AddParticipant(String),
    #[error("error removing a participant: {0}")]
    RemoveParticipant(String),
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
        SessionError::Processing(err.to_string())
    }
}
