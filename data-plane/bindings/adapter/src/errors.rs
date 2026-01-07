// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;

use slim_service::errors::ServiceError;
use slim_session::errors::SessionError;
use slim_auth::errors::AuthError;

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
