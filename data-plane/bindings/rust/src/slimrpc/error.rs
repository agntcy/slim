// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Status codes and error handling for SlimRPC
//!
//! This module provides gRPC-compatible status codes and error types.

use std::fmt;

/// gRPC status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, uniffi::Enum)]
#[repr(u16)]
pub enum RpcCode {
    /// Success
    #[default]
    Ok = 0,
    /// The operation was cancelled
    Cancelled = 1,
    /// Unknown error
    Unknown = 2,
    /// Client specified an invalid argument
    InvalidArgument = 3,
    /// Deadline exceeded before operation could complete
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
    /// Operation is not implemented or not supported
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

impl RpcCode {
    /// Returns true if this is a success code
    pub fn is_ok(&self) -> bool {
        matches!(self, RpcCode::Ok)
    }

    /// Returns true if this is an error code
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

impl fmt::Display for RpcCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RpcCode::Ok => "OK",
            RpcCode::Cancelled => "CANCELLED",
            RpcCode::Unknown => "UNKNOWN",
            RpcCode::InvalidArgument => "INVALID_ARGUMENT",
            RpcCode::DeadlineExceeded => "DEADLINE_EXCEEDED",
            RpcCode::NotFound => "NOT_FOUND",
            RpcCode::AlreadyExists => "ALREADY_EXISTS",
            RpcCode::PermissionDenied => "PERMISSION_DENIED",
            RpcCode::ResourceExhausted => "RESOURCE_EXHAUSTED",
            RpcCode::FailedPrecondition => "FAILED_PRECONDITION",
            RpcCode::Aborted => "ABORTED",
            RpcCode::OutOfRange => "OUT_OF_RANGE",
            RpcCode::Unimplemented => "UNIMPLEMENTED",
            RpcCode::Internal => "INTERNAL",
            RpcCode::Unavailable => "UNAVAILABLE",
            RpcCode::DataLoss => "DATA_LOSS",
            RpcCode::Unauthenticated => "UNAUTHENTICATED",
        };
        write!(f, "{}", s)
    }
}

impl From<RpcCode> for i32 {
    fn from(code: RpcCode) -> i32 {
        code as i32
    }
}

impl TryFrom<i32> for RpcCode {
    type Error = InvalidRpcCode;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(RpcCode::Ok),
            1 => Ok(RpcCode::Cancelled),
            2 => Ok(RpcCode::Unknown),
            3 => Ok(RpcCode::InvalidArgument),
            4 => Ok(RpcCode::DeadlineExceeded),
            5 => Ok(RpcCode::NotFound),
            6 => Ok(RpcCode::AlreadyExists),
            7 => Ok(RpcCode::PermissionDenied),
            8 => Ok(RpcCode::ResourceExhausted),
            9 => Ok(RpcCode::FailedPrecondition),
            10 => Ok(RpcCode::Aborted),
            11 => Ok(RpcCode::OutOfRange),
            12 => Ok(RpcCode::Unimplemented),
            13 => Ok(RpcCode::Internal),
            14 => Ok(RpcCode::Unavailable),
            15 => Ok(RpcCode::DataLoss),
            16 => Ok(RpcCode::Unauthenticated),
            _ => Err(InvalidRpcCode(value)),
        }
    }
}

/// Error returned when trying to convert an invalid i32 to RpcCode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRpcCode(pub i32);

impl fmt::Display for InvalidRpcCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid RPC code: {}", self.0)
    }
}

impl std::error::Error for InvalidRpcCode {}

/// UniFFI-compatible RPC error
///
/// This represents RPC errors with gRPC-compatible status codes.
#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
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
    pub fn new(code: RpcCode, message: impl Into<String>) -> Self {
        Self::Rpc {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Create a new RPC error with details
    pub fn with_details(code: RpcCode, message: impl Into<String>, details: Vec<u8>) -> Self {
        Self::Rpc {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    /// Create a RPC error with just a code
    pub fn with_code(code: RpcCode) -> Self {
        Self::Rpc {
            code,
            message: String::new(),
            details: None,
        }
    }

    /// Create an OK status (for completeness, though typically not used as an error)
    pub fn ok() -> Self {
        Self::with_code(RpcCode::Ok)
    }

    /// Create a cancelled error
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Cancelled, message)
    }

    /// Create an unknown error
    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Unknown, message)
    }

    /// Create an invalid argument error
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(RpcCode::InvalidArgument, message)
    }

    /// Create a deadline exceeded error
    pub fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self::new(RpcCode::DeadlineExceeded, message)
    }

    /// Create a not found error
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(RpcCode::NotFound, message)
    }

    /// Create an already exists error
    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(RpcCode::AlreadyExists, message)
    }

    /// Create a permission denied error
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(RpcCode::PermissionDenied, message)
    }

    /// Create a resource exhausted error
    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(RpcCode::ResourceExhausted, message)
    }

    /// Create a failed precondition error
    pub fn failed_precondition(message: impl Into<String>) -> Self {
        Self::new(RpcCode::FailedPrecondition, message)
    }

    /// Create an aborted error
    pub fn aborted(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Aborted, message)
    }

    /// Create an out of range error
    pub fn out_of_range(message: impl Into<String>) -> Self {
        Self::new(RpcCode::OutOfRange, message)
    }

    /// Create an unimplemented error
    pub fn unimplemented(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Unimplemented, message)
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Internal, message)
    }

    /// Create an unavailable error
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Unavailable, message)
    }

    /// Create a data loss error
    pub fn data_loss(message: impl Into<String>) -> Self {
        Self::new(RpcCode::DataLoss, message)
    }

    /// Create an unauthenticated error
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(RpcCode::Unauthenticated, message)
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

    /// Returns true if this is a success status
    pub fn is_ok(&self) -> bool {
        self.code().is_ok()
    }

    /// Returns true if this is an error status
    pub fn is_err(&self) -> bool {
        self.code().is_err()
    }

    /// Add details to an existing error
    pub fn with_details_added(mut self, details: Vec<u8>) -> Self {
        match &mut self {
            Self::Rpc { details: d, .. } => *d = Some(details),
        }
        self
    }
}

impl Default for RpcError {
    fn default() -> Self {
        Self::ok()
    }
}

impl From<RpcCode> for RpcError {
    fn from(code: RpcCode) -> Self {
        Self::with_code(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_conversions() {
        let ok_code: i32 = RpcCode::Ok.into();
        let internal_code: i32 = RpcCode::Internal.into();
        assert_eq!(ok_code, 0);
        assert_eq!(internal_code, 13);
        assert_eq!(RpcCode::try_from(0), Ok(RpcCode::Ok));
        assert_eq!(RpcCode::try_from(13), Ok(RpcCode::Internal));
        assert!(RpcCode::try_from(999).is_err());
    }

    #[test]
    fn test_code_display() {
        assert_eq!(RpcCode::Ok.to_string(), "OK");
        assert_eq!(RpcCode::NotFound.to_string(), "NOT_FOUND");
        assert_eq!(RpcCode::Internal.to_string(), "INTERNAL");
    }

    #[test]
    fn test_code_is_ok() {
        assert!(RpcCode::Ok.is_ok());
        assert!(!RpcCode::Internal.is_ok());
        assert!(!RpcCode::Ok.is_err());
        assert!(RpcCode::Internal.is_err());
    }

    #[test]
    fn test_error_creation() {
        let error = RpcError::ok();
        assert_eq!(error.code(), RpcCode::Ok);
        assert!(error.is_ok());

        let error = RpcError::internal("test error");
        assert_eq!(error.code(), RpcCode::Internal);
        assert!(error.is_err());
        assert_eq!(error.message(), "test error");
    }

    #[test]
    fn test_error_with_details() {
        let details = vec![1, 2, 3, 4];
        let error = RpcError::with_details(RpcCode::Internal, "error", details.clone());
        assert_eq!(error.details(), Some(details.as_slice()));
    }

    #[test]
    fn test_error_display() {
        let error = RpcError::ok();
        assert_eq!(error.to_string(), "");

        let error = RpcError::internal("test");
        assert_eq!(error.to_string(), "test");
    }

    #[test]
    fn test_error_from_code() {
        let error: RpcError = RpcCode::NotFound.into();
        assert_eq!(error.code(), RpcCode::NotFound);
        assert_eq!(error.message(), "");
    }

    #[test]
    fn test_all_error_constructors() {
        assert_eq!(RpcError::cancelled("msg").code(), RpcCode::Cancelled);
        assert_eq!(RpcError::unknown("msg").code(), RpcCode::Unknown);
        assert_eq!(
            RpcError::invalid_argument("msg").code(),
            RpcCode::InvalidArgument
        );
        assert_eq!(
            RpcError::deadline_exceeded("msg").code(),
            RpcCode::DeadlineExceeded
        );
        assert_eq!(RpcError::not_found("msg").code(), RpcCode::NotFound);
        assert_eq!(
            RpcError::already_exists("msg").code(),
            RpcCode::AlreadyExists
        );
        assert_eq!(
            RpcError::permission_denied("msg").code(),
            RpcCode::PermissionDenied
        );
        assert_eq!(
            RpcError::resource_exhausted("msg").code(),
            RpcCode::ResourceExhausted
        );
        assert_eq!(
            RpcError::failed_precondition("msg").code(),
            RpcCode::FailedPrecondition
        );
        assert_eq!(RpcError::aborted("msg").code(), RpcCode::Aborted);
        assert_eq!(RpcError::out_of_range("msg").code(), RpcCode::OutOfRange);
        assert_eq!(
            RpcError::unimplemented("msg").code(),
            RpcCode::Unimplemented
        );
        assert_eq!(RpcError::internal("msg").code(), RpcCode::Internal);
        assert_eq!(RpcError::unavailable("msg").code(), RpcCode::Unavailable);
        assert_eq!(RpcError::data_loss("msg").code(), RpcCode::DataLoss);
        assert_eq!(
            RpcError::unauthenticated("msg").code(),
            RpcCode::Unauthenticated
        );
    }
}
