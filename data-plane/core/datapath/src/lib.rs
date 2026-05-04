// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// On wasm, alias tokio_with_wasm as tokio so all data plane code can keep
// using `tokio::*` paths uniformly across native and wasm builds (matching
// the pattern used by `slim-session`).
#[cfg(all(feature = "wasm", not(feature = "native")))]
extern crate tokio_with_wasm as tokio;

pub mod api;
pub mod errors;
pub mod messages;
pub mod tables;

#[cfg(any(feature = "native", feature = "wasm"))]
pub mod message_processing;

#[cfg(any(feature = "native", feature = "wasm"))]
mod connection;
#[cfg(any(feature = "native", feature = "wasm"))]
mod forwarder;
#[cfg(any(feature = "native", feature = "wasm"))]
mod recovery;
#[cfg(any(feature = "native", feature = "wasm"))]
pub mod runtime;
#[cfg(any(feature = "native", feature = "wasm"))]
pub(crate) mod subscription_ack;
#[cfg(any(feature = "native", feature = "wasm"))]
mod websocket;

#[cfg(feature = "native")]
pub use tonic::Status;

/// Lightweight Status type for WASM builds where tonic is not available.
#[cfg(not(feature = "native"))]
#[derive(Debug, Clone)]
pub struct Status {
    code: u32,
    message: String,
}

#[cfg(not(feature = "native"))]
impl Status {
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(13, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(14, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(3, message)
    }

    pub fn code(&self) -> u32 {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[cfg(not(feature = "native"))]
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status(code={}, message={})", self.code, self.message)
    }
}

#[cfg(not(feature = "native"))]
impl std::error::Error for Status {}
