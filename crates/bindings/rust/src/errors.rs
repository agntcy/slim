// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;

use slim_auth::errors::AuthError;
use slim_service::errors::ServiceError;
use slim_session::errors::SessionError;

/// Error types for SLIM operations
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SlimError {
    #[error("service error: {message}")]
    ServiceError { message: String },
    #[error("Session error: {message}")]
    SessionError { message: String },
    #[error("Receive error: {message}")]
    ReceiveError { message: String },
    #[error("Send error: {message}")]
    SendError { message: String },
    #[error("Authentication error: {message}")]
    AuthError { message: String },
    #[error("Configuration error: {message}")]
    ConfigError { message: String },
    #[error("RPC error: {message}")]
    RpcError { message: String },
    #[error("Operation timed out")]
    Timeout,
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Internal error: {message}")]
    InternalError { message: String },
}

macro_rules! impl_from_error_for_slim {
    ($source:ty, $variant:ident) => {
        impl From<$source> for SlimError {
            fn from(err: $source) -> Self {
                SlimError::$variant {
                    message: err.chain().to_string(),
                }
            }
        }
    };
}

impl_from_error_for_slim!(ServiceError, ServiceError);
impl_from_error_for_slim!(SessionError, SessionError);
impl_from_error_for_slim!(AuthError, AuthError);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test SlimError Display implementations
    #[test]
    fn test_slim_error_display() {
        let errors = vec![
            SlimError::SessionError {
                message: "session".to_string(),
            },
            SlimError::ReceiveError {
                message: "receive".to_string(),
            },
            SlimError::SendError {
                message: "send".to_string(),
            },
            SlimError::AuthError {
                message: "auth".to_string(),
            },
            SlimError::ConfigError {
                message: "config".to_string(),
            },
            SlimError::RpcError {
                message: "rpc".to_string(),
            },
            SlimError::Timeout,
            SlimError::InvalidArgument {
                message: "invalid".to_string(),
            },
            SlimError::InternalError {
                message: "internal".to_string(),
            },
        ];

        for error in errors {
            let display = format!("{}", error);
            assert!(!display.is_empty(), "Error display should not be empty");
        }

        // Specific checks
        assert!(format!("{}", SlimError::Timeout).contains("timed out"));
    }
}
