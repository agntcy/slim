// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Status codes and error handling for SlimRPC
//!
//! This module provides gRPC-compatible status codes and error types.

use std::fmt;

/// gRPC status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, uniffi::Enum)]
#[repr(u16)]
pub enum Code {
    /// Success
    #[default]
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

impl Code {
    /// Returns true if this is a success code
    pub fn is_ok(&self) -> bool {
        matches!(self, Code::Ok)
    }

    /// Returns true if this is an error code
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    /// Convert from i32
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Code::Ok),
            1 => Some(Code::Cancelled),
            2 => Some(Code::Unknown),
            3 => Some(Code::InvalidArgument),
            4 => Some(Code::DeadlineExceeded),
            5 => Some(Code::NotFound),
            6 => Some(Code::AlreadyExists),
            7 => Some(Code::PermissionDenied),
            8 => Some(Code::ResourceExhausted),
            9 => Some(Code::FailedPrecondition),
            10 => Some(Code::Aborted),
            11 => Some(Code::OutOfRange),
            12 => Some(Code::Unimplemented),
            13 => Some(Code::Internal),
            14 => Some(Code::Unavailable),
            15 => Some(Code::DataLoss),
            16 => Some(Code::Unauthenticated),
            _ => None,
        }
    }

    /// Convert to i32
    pub fn as_i32(&self) -> i32 {
        *self as i32
    }

    /// Get the string representation of this code
    pub fn as_str(&self) -> &'static str {
        match self {
            Code::Ok => "OK",
            Code::Cancelled => "CANCELLED",
            Code::Unknown => "UNKNOWN",
            Code::InvalidArgument => "INVALID_ARGUMENT",
            Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Code::NotFound => "NOT_FOUND",
            Code::AlreadyExists => "ALREADY_EXISTS",
            Code::PermissionDenied => "PERMISSION_DENIED",
            Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
            Code::FailedPrecondition => "FAILED_PRECONDITION",
            Code::Aborted => "ABORTED",
            Code::OutOfRange => "OUT_OF_RANGE",
            Code::Unimplemented => "UNIMPLEMENTED",
            Code::Internal => "INTERNAL",
            Code::Unavailable => "UNAVAILABLE",
            Code::DataLoss => "DATA_LOSS",
            Code::Unauthenticated => "UNAUTHENTICATED",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<Code> for i32 {
    fn from(code: Code) -> i32 {
        code.as_i32()
    }
}

impl TryFrom<i32> for Code {
    type Error = StatusError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Code::from_i32(value).ok_or(StatusError::InvalidCode(value))
    }
}

/// RPC status with code and optional message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Status code
    code: Code,
    /// Optional status message
    message: Option<String>,
    /// Optional binary details (for protobuf Any types)
    details: Option<Vec<u8>>,
}

impl Status {
    /// Create a new status
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: Some(message.into()),
            details: None,
        }
    }

    /// Create a status with just a code
    pub fn with_code(code: Code) -> Self {
        Self {
            code,
            message: None,
            details: None,
        }
    }

    /// Create a success status
    pub fn ok() -> Self {
        Self::with_code(Code::Ok)
    }

    /// Create a cancelled status
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(Code::Cancelled, message)
    }

    /// Create an unknown error status
    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(Code::Unknown, message)
    }

    /// Create an invalid argument status
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidArgument, message)
    }

    /// Create a deadline exceeded status
    pub fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self::new(Code::DeadlineExceeded, message)
    }

    /// Create a not found status
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Code::NotFound, message)
    }

    /// Create an already exists status
    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(Code::AlreadyExists, message)
    }

    /// Create a permission denied status
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(Code::PermissionDenied, message)
    }

    /// Create a resource exhausted status
    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(Code::ResourceExhausted, message)
    }

    /// Create a failed precondition status
    pub fn failed_precondition(message: impl Into<String>) -> Self {
        Self::new(Code::FailedPrecondition, message)
    }

    /// Create an aborted status
    pub fn aborted(message: impl Into<String>) -> Self {
        Self::new(Code::Aborted, message)
    }

    /// Create an out of range status
    pub fn out_of_range(message: impl Into<String>) -> Self {
        Self::new(Code::OutOfRange, message)
    }

    /// Create an unimplemented status
    pub fn unimplemented(message: impl Into<String>) -> Self {
        Self::new(Code::Unimplemented, message)
    }

    /// Create an internal error status
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Code::Internal, message)
    }

    /// Create an unavailable status
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Code::Unavailable, message)
    }

    /// Create a data loss status
    pub fn data_loss(message: impl Into<String>) -> Self {
        Self::new(Code::DataLoss, message)
    }

    /// Create an unauthenticated status
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(Code::Unauthenticated, message)
    }

    /// Get the status code
    pub fn code(&self) -> Code {
        self.code
    }

    /// Get the status message
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Get the status details
    pub fn details(&self) -> Option<&[u8]> {
        self.details.as_deref()
    }

    /// Set details
    pub fn with_details(mut self, details: Vec<u8>) -> Self {
        self.details = Some(details);
        self
    }

    /// Returns true if this is a success status
    pub fn is_ok(&self) -> bool {
        self.code.is_ok()
    }

    /// Returns true if this is an error status
    pub fn is_err(&self) -> bool {
        self.code.is_err()
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::ok()
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Status {{ code: {}", self.code)?;
        if let Some(msg) = &self.message {
            write!(f, ", message: \"{}\"", msg)?;
        }
        write!(f, " }}")
    }
}

impl std::error::Error for Status {}

impl From<Code> for Status {
    fn from(code: Code) -> Self {
        Self::with_code(code)
    }
}

/// Errors that can occur when working with Status
#[derive(Debug, thiserror::Error)]
pub enum StatusError {
    /// Invalid status code
    #[error("Invalid status code: {0}")]
    InvalidCode(i32),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}

/// UniFFI-compatible RPC error
///
/// This wraps Status to make it compatible with UniFFI foreign language bindings.
/// UniFFI requires errors to be represented as enums with associated data.
#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum RpcError {
    #[error("{message}")]
    Rpc {
        code: Code,
        message: String,
        details: Option<Vec<u8>>,
    },
}

impl RpcError {
    /// Create a new RPC error
    pub fn new(code: Code, message: String) -> Self {
        Self::Rpc {
            code,
            message,
            details: None,
        }
    }

    /// Create a new RPC error with details
    pub fn with_details(code: Code, message: String, details: Vec<u8>) -> Self {
        Self::Rpc {
            code,
            message,
            details: Some(details),
        }
    }

    /// Get the error code
    pub fn code(&self) -> Code {
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
            code: status.code(),
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
                let mut status = Status::new(code, message);
                if let Some(d) = details {
                    status = status.with_details(d);
                }
                status
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_conversions() {
        assert_eq!(Code::Ok.as_i32(), 0);
        assert_eq!(Code::Internal.as_i32(), 13);
        assert_eq!(Code::from_i32(0), Some(Code::Ok));
        assert_eq!(Code::from_i32(13), Some(Code::Internal));
        assert_eq!(Code::from_i32(999), None);
    }

    #[test]
    fn test_code_display() {
        assert_eq!(Code::Ok.to_string(), "OK");
        assert_eq!(Code::NotFound.to_string(), "NOT_FOUND");
        assert_eq!(Code::Internal.to_string(), "INTERNAL");
    }

    #[test]
    fn test_code_is_ok() {
        assert!(Code::Ok.is_ok());
        assert!(!Code::Internal.is_ok());
        assert!(!Code::Ok.is_err());
        assert!(Code::Internal.is_err());
    }

    #[test]
    fn test_status_creation() {
        let status = Status::ok();
        assert_eq!(status.code(), Code::Ok);
        assert!(status.is_ok());

        let status = Status::internal("test error");
        assert_eq!(status.code(), Code::Internal);
        assert!(status.is_err());
        assert_eq!(status.message(), Some("test error"));
    }

    #[test]
    fn test_status_with_details() {
        let details = vec![1, 2, 3, 4];
        let status = Status::internal("error").with_details(details.clone());
        assert_eq!(status.details(), Some(details.as_slice()));
    }

    #[test]
    fn test_status_display() {
        let status = Status::ok();
        assert_eq!(status.to_string(), "Status { code: OK }");

        let status = Status::internal("test");
        assert_eq!(
            status.to_string(),
            "Status { code: INTERNAL, message: \"test\" }"
        );
    }

    #[test]
    fn test_status_from_code() {
        let status: Status = Code::NotFound.into();
        assert_eq!(status.code(), Code::NotFound);
        assert_eq!(status.message(), None);
    }

    #[test]
    fn test_all_status_constructors() {
        assert_eq!(Status::cancelled("msg").code(), Code::Cancelled);
        assert_eq!(Status::unknown("msg").code(), Code::Unknown);
        assert_eq!(
            Status::invalid_argument("msg").code(),
            Code::InvalidArgument
        );
        assert_eq!(
            Status::deadline_exceeded("msg").code(),
            Code::DeadlineExceeded
        );
        assert_eq!(Status::not_found("msg").code(), Code::NotFound);
        assert_eq!(Status::already_exists("msg").code(), Code::AlreadyExists);
        assert_eq!(
            Status::permission_denied("msg").code(),
            Code::PermissionDenied
        );
        assert_eq!(
            Status::resource_exhausted("msg").code(),
            Code::ResourceExhausted
        );
        assert_eq!(
            Status::failed_precondition("msg").code(),
            Code::FailedPrecondition
        );
        assert_eq!(Status::aborted("msg").code(), Code::Aborted);
        assert_eq!(Status::out_of_range("msg").code(), Code::OutOfRange);
        assert_eq!(Status::unimplemented("msg").code(), Code::Unimplemented);
        assert_eq!(Status::internal("msg").code(), Code::Internal);
        assert_eq!(Status::unavailable("msg").code(), Code::Unavailable);
        assert_eq!(Status::data_loss("msg").code(), Code::DataLoss);
        assert_eq!(Status::unauthenticated("msg").code(), Code::Unauthenticated);
    }
}
