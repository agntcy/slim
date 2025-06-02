// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigurationError {
    #[error("configuration error: {0}")]
    ConfigError(String),
    #[error("unknown error")]
    Unknown,
}

pub trait Configuration {
    /// Validate the component configuration
    fn validate(&self) -> Result<(), ConfigurationError>;
}
