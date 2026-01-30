// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Error types for SlimRPC UniFFI bindings
//!
//! Provides UniFFI-compatible error types that wrap the core SlimRPC error types.

// Re-export core types
pub use slim_rpc::{Code, Status, StatusError};

/// UniFFI-compatible RPC error
///
/// This wraps the core SlimRPC Status type to make it compatible with UniFFI.
/// UniFFI requires errors to be represented as enums with associated data.
#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
//#[uniffi(flat_error)]
pub enum RpcError {
    #[error("{message}")]
    Rpc {
        code: RpcCode,
        message: String,
        details: Option<Vec<u8>>,
    },
}

impl RpcError {
    /// Create a new RPC error
    pub fn new(code: RpcCode, message: String) -> Self {
        Self::Rpc {
            code,
            message,
            details: None,
        }
    }

    /// Create a new RPC error with details
    pub fn with_details(code: RpcCode, message: String, details: Vec<u8>) -> Self {
        Self::Rpc {
            code,
            message,
            details: Some(details),
        }
    }

    /// Get the error code
    pub fn code(&self) -> RpcCode {
        match self {
            Self::Rpc { code, .. } => *code,
        }
    }

    /// Get the error message
    pub fn message(&self) -> &str {
        match self {
            Self::Rpc { message, .. } => message,
        }
    }

    /// Get the error details
    pub fn details(&self) -> Option<&[u8]> {
        match self {
            Self::Rpc { details, .. } => details.as_deref(),
        }
    }
}

impl From<Status> for RpcError {
    fn from(status: Status) -> Self {
        Self::Rpc {
            code: status.code().into(),
            message: status.message().unwrap_or("").to_string(),
            details: status.details().map(|d| d.to_vec()),
        }
    }
}

impl From<RpcError> for Status {
    fn from(error: RpcError) -> Self {
        match error {
            RpcError::Rpc {
                code,
                message,
                details,
            } => {
                let mut status = Status::new(code.into(), message);
                if let Some(d) = details {
                    status = status.with_details(d);
                }
                status
            }
        }
    }
}

/// gRPC-compatible status codes
///
/// UniFFI-compatible version of the gRPC status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
#[repr(u16)]
pub enum RpcCode {
    /// Not an error; returned on success
    Ok = 0,
    /// The operation was cancelled
    Cancelled = 1,
    /// Unknown error
    Unknown = 2,
    /// Client specified an invalid argument
    InvalidArgument = 3,
    /// Deadline expired before operation could complete
    DeadlineExceeded = 4,
    /// Some requested entity was not found
    NotFound = 5,
    /// Some entity that we attempted to create already exists
    AlreadyExists = 6,
    /// The caller does not have permission to execute the specified operation
    PermissionDenied = 7,
    /// Some resource has been exhausted
    ResourceExhausted = 8,
    /// The system is not in a state required for the operation's execution
    FailedPrecondition = 9,
    /// The operation was aborted
    Aborted = 10,
    /// Operation was attempted past the valid range
    OutOfRange = 11,
    /// Operation is not implemented or not supported/enabled
    Unimplemented = 12,
    /// Internal errors
    Internal = 13,
    /// The service is currently unavailable
    Unavailable = 14,
    /// Unrecoverable data loss or corruption
    DataLoss = 15,
    /// The request does not have valid authentication credentials
    Unauthenticated = 16,
}

impl From<Code> for RpcCode {
    fn from(code: Code) -> Self {
        match code {
            Code::Ok => RpcCode::Ok,
            Code::Cancelled => RpcCode::Cancelled,
            Code::Unknown => RpcCode::Unknown,
            Code::InvalidArgument => RpcCode::InvalidArgument,
            Code::DeadlineExceeded => RpcCode::DeadlineExceeded,
            Code::NotFound => RpcCode::NotFound,
            Code::AlreadyExists => RpcCode::AlreadyExists,
            Code::PermissionDenied => RpcCode::PermissionDenied,
            Code::ResourceExhausted => RpcCode::ResourceExhausted,
            Code::FailedPrecondition => RpcCode::FailedPrecondition,
            Code::Aborted => RpcCode::Aborted,
            Code::OutOfRange => RpcCode::OutOfRange,
            Code::Unimplemented => RpcCode::Unimplemented,
            Code::Internal => RpcCode::Internal,
            Code::Unavailable => RpcCode::Unavailable,
            Code::DataLoss => RpcCode::DataLoss,
            Code::Unauthenticated => RpcCode::Unauthenticated,
        }
    }
}

impl From<RpcCode> for Code {
    fn from(code: RpcCode) -> Self {
        match code {
            RpcCode::Ok => Code::Ok,
            RpcCode::Cancelled => Code::Cancelled,
            RpcCode::Unknown => Code::Unknown,
            RpcCode::InvalidArgument => Code::InvalidArgument,
            RpcCode::DeadlineExceeded => Code::DeadlineExceeded,
            RpcCode::NotFound => Code::NotFound,
            RpcCode::AlreadyExists => Code::AlreadyExists,
            RpcCode::PermissionDenied => Code::PermissionDenied,
            RpcCode::ResourceExhausted => Code::ResourceExhausted,
            RpcCode::FailedPrecondition => Code::FailedPrecondition,
            RpcCode::Aborted => Code::Aborted,
            RpcCode::OutOfRange => Code::OutOfRange,
            RpcCode::Unimplemented => Code::Unimplemented,
            RpcCode::Internal => Code::Internal,
            RpcCode::Unavailable => Code::Unavailable,
            RpcCode::DataLoss => Code::DataLoss,
            RpcCode::Unauthenticated => Code::Unauthenticated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        let status = Status::new(Code::NotFound, "Resource not found");
        let error: RpcError = status.into();

        assert_eq!(error.code(), RpcCode::NotFound);
        assert_eq!(error.message(), "Resource not found");
    }

    #[test]
    fn test_code_conversion() {
        let rpc_code = RpcCode::InvalidArgument;
        let code: Code = rpc_code.into();
        let back: RpcCode = code.into();

        assert_eq!(rpc_code, back);
    }

    #[test]
    fn test_error_with_details() {
        let details = vec![1, 2, 3, 4];
        let error = RpcError::with_details(
            RpcCode::Internal,
            "Internal error".to_string(),
            details.clone(),
        );

        assert_eq!(error.code(), RpcCode::Internal);
        assert_eq!(error.message(), "Internal error");
        assert_eq!(error.details(), Some(details.as_slice()));
    }
}
