// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;
use slim_auth::errors::AuthError;
use slim_datapath::errors::DataPathError;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("datapath error: {0}")]
    Datapath(#[from] DataPathError),
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),
    #[error("payload missing")]
    PayloadMissing,
}
