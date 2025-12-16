// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::errors::AuthError;
use slim_config::grpc::errors::ConfigError;
use slim_datapath::errors::DataPathError;
use thiserror::Error;
use tonic::Status;

#[derive(Error, Debug)]
pub enum ControllerError {
    // Configuration / setup
    #[error("configuration error")]
    ConfigError(#[from] ConfigError),

    // Connection lifecycle
    #[error("controller already started")]
    AlreadyStarted,
    #[error("controller already stopped")]
    AlreadyStopped,
    #[error("timeout waiting for shutdown to complete")]
    ShutdownTimeout,
    #[error("grpc error")]
    GrpcError(#[from] Status),

    // Clients / Servers errors
    #[error("client already running: {0}")]
    ClientAlreadyRunning(String),
    #[error("server already running: {0}")]
    ServerAlreadyRunning(String),

    #[error("malformed name: {0}")]
    MalformedName(String),

    // Propagated lower-level errors
    #[error("datapath error")]
    Datapath(#[from] DataPathError),
    #[error("error sending message to data plane: {0}")]
    DatapathSendError(String),
    #[error("auth error")]
    Auth(#[from] AuthError),
    #[error("message error")]
    MessageError(#[from] slim_datapath::messages::utils::MessageError),

    // Payload / validation
    #[error("payload missing")]
    PayloadMissing,
}
