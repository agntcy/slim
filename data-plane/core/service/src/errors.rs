// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::errors::AuthError;
use slim_config::component::id::IdError;
use thiserror::Error;

#[cfg(feature = "session")]
use slim_session::errors::SessionError as SessionErrorType;

#[derive(Error, Debug)]
pub enum SubscriptionAckError {
    #[error("ack rejected by datapath: {message}")]
    Rejected { message: String },
    #[error("ack channel closed")]
    ChannelClosed,
}

#[derive(Error, Debug)]
pub enum ServiceError {
    // Configuration / setup
    #[error("no server or client configured")]
    NoServerOrClientConfigured,
    #[error("grpc configuration error")]
    GrpcConfigError(#[from] slim_config::grpc::errors::ConfigError),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("id error")]
    IdError(#[from] IdError),

    // App construction
    #[error("application name missing")]
    NoAppName,
    #[error("identity provider missing")]
    NoIdentityProvider,
    #[error("identity verifier missing")]
    NoIdentityVerifier,
    #[error("app already registered")]
    AppAlreadyRegistered,
    #[error("app not found: {0}")]
    AppNotFound(String),

    // Auth
    #[error("provider auth error")]
    ProviderAuthError(#[from] AuthError),

    // Connection lifecycle
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("disconnect error: {0}")]
    DisconnectError(String),
    #[error("client already connected: {0}")]
    ClientAlreadyConnected(String),
    #[error("server not found: {0}")]
    ServerNotFound(String),
    #[error("connection not found: {0}")]
    ConnectionNotFoundForEndpoint(String),
    #[error("different id found for connection {endpoint}: expected {expected}, found {found}")]
    DifferentIdForConnection {
        endpoint: String,
        expected: u64,
        found: u64,
    },

    // Routing / subscription operations
    #[error("error sending subscription")]
    SubscriptionError(#[source] SubscriptionAckError),
    #[error("error sending unsubscription")]
    UnsubscriptionError(#[source] SubscriptionAckError),
    #[error("error on set route: {0}")]
    SetRouteError(String),
    #[error("error on remove route: {0}")]
    RemoveRouteError(String),

    // Messaging
    #[error("error publishing message: {0}")]
    PublishError(String),
    // Structured receive-related variants replacing legacy ReceiveError(String)
    #[error("receive timeout waiting for item")]
    ReceiveTimeout,
    #[error("receive channel closed")]
    ReceiveChannelClosed,
    #[error("message decode failure: {0}")]
    ReceiveDecodeFailure(String),
    #[error("error sending message: {0}")]
    MessageSendingError(String),

    // Session related
    #[cfg(feature = "session")]
    #[error("session not found")]
    SessionNotFound,
    #[cfg(feature = "session")]
    #[error("to be able to call invite/remove, session must be multicast: {0}")]
    SessionMustBeMulticast(String),
    #[cfg(feature = "session")]
    #[error("error in session")]
    SessionError(#[from] SessionErrorType),

    // Controller / datapath typed propagation
    #[error("controller error")]
    Controller(#[from] slim_controller::errors::ControllerError),
    #[error("datapath error")]
    DataPath(#[from] slim_datapath::errors::DataPathError),

    // Storage
    #[error("storage error: {0}")]
    StorageError(String), // legacy string variant
    #[error("storage I/O error")]
    StorageIo(#[from] std::io::Error),
    #[error("storage home directory unavailable")]
    HomeDirUnavailable,

    // Drain / shutdown
    #[error("drain signal missing")]
    NoDrainSignal,
    #[error("timed out while waiting for sessions to close")]
    DrainTimeoutError,

    // Catch-all
    #[error("unknown error")]
    Unknown,
}
