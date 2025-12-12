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
    #[error("configuration error {0}")]
    ConfigError(#[from] ConfigError),

    // Connection lifecycle
    #[error("controller already started")]
    AlreadyStarted,
    #[error("controller already stopped")]
    AlreadyStopped,
    #[error("timeout waiting for shutdown to complete")]
    ShutdownTimeout,
    #[error("grpc error: {0}")]
    GrpcError(#[from] Status),

    // Clients / Servers errors
    #[error("client already running: {0}")]
    ClientAlreadyRunning(String),
    #[error("server already running: {0}")]
    ServerAlreadyRunning(String),

    #[error("malformed name: {0}")]
    MalformedName(String),

    // Propagated lower-level errors
    #[error("datapath error: {0}")]
    Datapath(#[from] DataPathError),
    #[error("error sending message to data plane: {0}")]
    DatapathSendError(String),
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    // Payload / validation
    #[error("payload missing")]
    PayloadMissing,
}
