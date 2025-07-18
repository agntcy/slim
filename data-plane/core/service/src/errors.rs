// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

use crate::session::SessionMessage;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("agent already registered")]
    AgentAlreadyRegistered,
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("disconnect error: {0}")]
    DisconnectError(String),
    #[error("error sending subscription: {0}")]
    SubscriptionError(String),
    #[error("error sending unsubscription: {0}")]
    UnsubscriptionError(String),
    #[error("error on set route: {0}")]
    SetRouteError(String),
    #[error("error on remove route: {0}")]
    RemoveRouteError(String),
    #[error("error publishing message: {0}")]
    PublishError(String),
    #[error("error receiving message: {0}")]
    ReceiveError(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("error in session: {0}")]
    SessionError(String),
    #[error("client already connected: {0}")]
    ClientAlreadyConnected(String),
    #[error("server not found: {0}")]
    ServerNotFound(String),
    #[error("error sending message: {0}")]
    MessageSendingError(String),
    #[error("error in controller: {0}")]
    ControllerError(String),
    #[error("storage error: {0}")]
    StorageError(String),
    #[error("unknown error")]
    Unknown,
}

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
    #[error("session id already used: {0}")]
    SessionIdAlreadyUsed(String),
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("missing SLIM header: {0}")]
    MissingSlimHeader(String),
    #[error("missing session header")]
    MissingSessionHeader,
    #[error("session unknown: {0}")]
    SessionUnknown(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
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
        message: Box<SessionMessage>,
    },
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

    // Channel Endpoint errors
    #[error("error initializing MLS: {0}")]
    MLSInit(String),
    #[error("msl state is None")]
    NoMls,
    #[error("error generating key package: {0}")]
    MLSKeyPackage(String),
    #[error("invialid id message: {0}")]
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
