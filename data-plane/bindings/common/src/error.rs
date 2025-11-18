// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Error types for Go bindings

use std::fmt;

/// Result type for Go bindings
pub type Result<T> = std::result::Result<T, SlimError>;

/// Error types exposed to Go
#[derive(Debug)]
pub enum SlimError {
    ConfigError { message: String },
    SessionError { message: String },
    ReceiveError { message: String },
    SendError { message: String },
    AuthError { message: String },
    Timeout { message: String },
    InvalidArgument { message: String },
    InternalError { message: String },
}

impl fmt::Display for SlimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlimError::ConfigError { message } => write!(f, "Configuration error: {}", message),
            SlimError::SessionError { message } => write!(f, "Session error: {}", message),
            SlimError::ReceiveError { message } => write!(f, "Receive error: {}", message),
            SlimError::SendError { message } => write!(f, "Send error: {}", message),
            SlimError::AuthError { message } => write!(f, "Authentication error: {}", message),
            SlimError::Timeout { message } => write!(f, "Operation timed out: {}", message),
            SlimError::InvalidArgument { message } => write!(f, "Invalid argument: {}", message),
            SlimError::InternalError { message } => write!(f, "Internal error: {}", message),
        }
    }
}

impl std::error::Error for SlimError {}

