// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SRPCError {
    #[error("RPC response error: code={0}, message={1}")]
    ResponseError(i32, String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("No handler found for: {0}")]
    NoHandler(String),

    #[error("Invalid ID format: {0}")]
    InvalidId(String),

    #[error("Invalid service method format: {0}")]
    InvalidServiceMethod(String),

    #[error("Invalid base name: {0}")]
    InvalidBaseName(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Failed to parse identity: {0}")]
    ParseIdentity(String),

    #[error("Failed to create shared secret auth: {0}")]
    AuthCreation(#[source] slim_auth::errors::AuthError),

    #[error("Failed to create app: {0}")]
    AppCreation(#[source] slim_service::errors::ServiceError),

    #[error("Failed to connect to SLIM service: {0}")]
    Connection(#[source] slim_service::errors::ServiceError),

    #[error("Failed to subscribe: {0}")]
    Subscription(#[source] slim_service::errors::ServiceError),

    #[error("Failed to publish: {0}")]
    PublishError(#[source] slim_session::SessionError),

    #[error("Failed to set route: {0}")]
    SetRoute(#[source] slim_service::errors::ServiceError),

    #[error("Failed to create session: {0}")]
    SessionCreationError(#[source] slim_session::SessionError),

    #[error("Failed to initialize session: {0}")]
    SessionInit(#[source] slim_session::SessionError),
}

pub type Result<T> = std::result::Result<T, SRPCError>;
