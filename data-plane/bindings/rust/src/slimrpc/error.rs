// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Status and error types for UniFFI
//!
//! Provides UniFFI-compatible wrappers around the core SlimRPC status types.

use std::fmt;

/// gRPC-compatible status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
#[repr(i32)]
pub enum Code {
    /// Success
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

impl Default for Code {
    fn default() -> Self {
        Code::Ok
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

impl From<agntcy_slimrpc::Code> for Code {
    fn from(code: agntcy_slimrpc::Code) -> Self {
        match code {
            agntcy_slimrpc::Code::Ok => Code::Ok,
            agntcy_slimrpc::Code::Cancelled => Code::Cancelled,
            agntcy_slimrpc::Code::Unknown => Code::Unknown,
            agntcy_slimrpc::Code::InvalidArgument => Code::InvalidArgument,
            agntcy_slimrpc::Code::DeadlineExceeded => Code::DeadlineExceeded,
            agntcy_slimrpc::Code::NotFound => Code::NotFound,
            agntcy_slimrpc::Code::AlreadyExists => Code::AlreadyExists,
            agntcy_slimrpc::Code::PermissionDenied => Code::PermissionDenied,
            agntcy_slimrpc::Code::ResourceExhausted => Code::ResourceExhausted,
            agntcy_slimrpc::Code::FailedPrecondition => Code::FailedPrecondition,
            agntcy_slimrpc::Code::Aborted => Code::Aborted,
            agntcy_slimrpc::Code::OutOfRange => Code::OutOfRange,
            agntcy_slimrpc::Code::Unimplemented => Code::Unimplemented,
            agntcy_slimrpc::Code::Internal => Code::Internal,
            agntcy_slimrpc::Code::Unavailable => Code::Unavailable,
            agntcy_slimrpc::Code::DataLoss => Code::DataLoss,
            agntcy_slimrpc::Code::Unauthenticated => Code::Unauthenticated,
        }
    }
}

impl From<Code> for agntcy_slimrpc::Code {
    fn from(code: Code) -> Self {
        match code {
            Code::Ok => agntcy_slimrpc::Code::Ok,
            Code::Cancelled => agntcy_slimrpc::Code::Cancelled,
            Code::Unknown => agntcy_slimrpc::Code::Unknown,
            Code::InvalidArgument => agntcy_slimrpc::Code::InvalidArgument,
            Code::DeadlineExceeded => agntcy_slimrpc::Code::DeadlineExceeded,
            Code::NotFound => agntcy_slimrpc::Code::NotFound,
            Code::AlreadyExists => agntcy_slimrpc::Code::AlreadyExists,
            Code::PermissionDenied => agntcy_slimrpc::Code::PermissionDenied,
            Code::ResourceExhausted => agntcy_slimrpc::Code::ResourceExhausted,
            Code::FailedPrecondition => agntcy_slimrpc::Code::FailedPrecondition,
            Code::Aborted => agntcy_slimrpc::Code::Aborted,
            Code::OutOfRange => agntcy_slimrpc::Code::OutOfRange,
            Code::Unimplemented => agntcy_slimrpc::Code::Unimplemented,
            Code::Internal => agntcy_slimrpc::Code::Internal,
            Code::Unavailable => agntcy_slimrpc::Code::Unavailable,
            Code::DataLoss => agntcy_slimrpc::Code::DataLoss,
            Code::Unauthenticated => agntcy_slimrpc::Code::Unauthenticated,
        }
    }
}

/// RPC status with code and optional message
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Status {
    /// Status code
    pub code: Code,
    /// Optional status message
    pub message: Option<String>,
}

impl Status {
    /// Create a new status
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: Some(message.into()),
        }
    }

    /// Create a status with just a code
    pub fn with_code(code: Code) -> Self {
        Self {
            code,
            message: None,
        }
    }

    /// Create a success status
    pub fn ok() -> Self {
        Self::with_code(Code::Ok)
    }

    /// Returns true if this is a success status
    pub fn is_ok(&self) -> bool {
        self.code.is_ok()
    }

    /// Returns true if this is an error status
    pub fn is_err(&self) -> bool {
        self.code.is_err()
    }

    /// Convert to core Status
    pub fn to_core(&self) -> agntcy_slimrpc::Status {
        if let Some(ref msg) = self.message {
            agntcy_slimrpc::Status::new(self.code.into(), msg.clone())
        } else {
            agntcy_slimrpc::Status::with_code(self.code.into())
        }
    }

    /// Convert to core Status (alias for to_core, for consistency)
    pub fn into_core_status(self) -> agntcy_slimrpc::Status {
        self.to_core()
    }

    /// Convert from core Status
    pub fn from_core(status: agntcy_slimrpc::Status) -> Self {
        Self {
            code: status.code().into(),
            message: status.message().map(|s| s.to_string()),
        }
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

impl From<Code> for Status {
    fn from(code: Code) -> Self {
        Self::with_code(code)
    }
}

impl From<agntcy_slimrpc::Status> for Status {
    fn from(status: agntcy_slimrpc::Status) -> Self {
        Self::from_core(status)
    }
}

/// RPC-specific error type
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum RpcError {
    /// RPC call failed with a status
    RpcStatus { status: Status },
    /// Invalid status code
    InvalidCode { code: i32 },
    /// Serialization error
    SerializationError { message: String },
    /// Deserialization error
    DeserializationError { message: String },
    /// Internal error
    InternalError { message: String },
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::RpcStatus { status } => write!(f, "RPC failed: {}", status),
            RpcError::InvalidCode { code } => write!(f, "Invalid status code: {}", code),
            RpcError::SerializationError { message } => {
                write!(f, "Serialization error: {}", message)
            }
            RpcError::DeserializationError { message } => {
                write!(f, "Deserialization error: {}", message)
            }
            RpcError::InternalError { message } => write!(f, "Internal error: {}", message),
        }
    }
}

impl std::error::Error for RpcError {}

impl From<agntcy_slimrpc::Status> for RpcError {
    fn from(status: agntcy_slimrpc::Status) -> Self {
        RpcError::RpcStatus {
            status: Status::from_core(status),
        }
    }
}

/// Status error type (for compatibility)
pub type StatusError = RpcError;

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
        assert_eq!(status.code, Code::Ok);
        assert!(status.is_ok());

        let status = Status::new(Code::Internal, "test error");
        assert_eq!(status.code, Code::Internal);
        assert!(status.is_err());
        assert_eq!(status.message.as_deref(), Some("test error"));
    }

    #[test]
    fn test_status_display() {
        let status = Status::ok();
        assert_eq!(status.to_string(), "Status { code: OK }");

        let status = Status::new(Code::Internal, "test");
        assert_eq!(
            status.to_string(),
            "Status { code: INTERNAL, message: \"test\" }"
        );
    }

    #[test]
    fn test_status_from_code() {
        let status: Status = Code::NotFound.into();
        assert_eq!(status.code, Code::NotFound);
        assert_eq!(status.message, None);
    }

    #[test]
    fn test_core_status_conversion() {
        let core_status = agntcy_slimrpc::Status::internal("test error");
        let status = Status::from_core(core_status);
        assert_eq!(status.code, Code::Internal);
        assert_eq!(status.message.as_deref(), Some("test error"));

        let back_to_core = status.to_core();
        assert_eq!(back_to_core.code(), agntcy_slimrpc::Code::Internal);
        assert_eq!(back_to_core.message(), Some("test error"));
    }
}