// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::errors::AuthError;
use slim_datapath::errors::DataPathError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    // Configuration / setup
    #[error("configuration error {0}")]
    ConfigError(String),

    // Connection lifecycle
    #[error("connection error: {0}")]
    ConnectionError(String),

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
