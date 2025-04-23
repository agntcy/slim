// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("datapath error: {0}")]
    DatapathError(String),
}
