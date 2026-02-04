// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Context types for SlimRPC UniFFI bindings
//!
//! Provides UniFFI-compatible wrappers around the core SlimRPC context types.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use crate::slimrpc_core::Context as SlimContext;

use crate::Name;

/// RPC context passed to handlers
///
/// UniFFI-compatible wrapper around the core SlimRPC Context.
/// Contains session information, metadata, and deadline information.
#[derive(Clone, uniffi::Object)]
pub struct Context {
    /// Wrapped core context
    inner: SlimContext,
}

#[uniffi::export]
impl Context {
    /// Get the session context
    pub fn session(&self) -> Arc<SessionContext> {
        Arc::new(SessionContext {
            inner: self.inner.session().clone(),
        })
    }

    /// Get the message context (if available)
    pub fn message(&self) -> Option<Arc<RpcMessageContext>> {
        self.inner.message().map(|m| {
            Arc::new(RpcMessageContext {
                source: m.source().clone().into(),
                destination: m.destination().map(|n| n.clone().into()),
                payload_type: m.payload_type().to_string(),
                metadata: m.metadata().as_map().clone(),
            })
        })
    }

    /// Get all metadata as a map
    pub fn metadata(&self) -> HashMap<String, String> {
        self.inner.metadata().as_map().clone()
    }

    /// Get a specific metadata value
    pub fn get_metadata(&self, key: String) -> Option<String> {
        self.inner.metadata().get(&key).map(|s| s.to_string())
    }

    /// Get the deadline for this RPC call (seconds since UNIX epoch)
    pub fn deadline(&self) -> Option<f64> {
        self.inner.deadline().map(|d| {
            d.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        })
    }

    /// Get the remaining time until deadline in seconds
    pub fn remaining_time(&self) -> Option<f64> {
        self.inner.remaining_time().map(|d| d.as_secs_f64())
    }

    /// Check if the deadline has been exceeded
    pub fn is_deadline_exceeded(&self) -> bool {
        self.inner.is_deadline_exceeded()
    }
}

impl Context {
    /// Create from core context
    pub(crate) fn from_inner(inner: SlimContext) -> Self {
        Self { inner }
    }
}

/// Session-level context information
///
/// Contains information about the RPC session including source, destination,
/// and session metadata.
#[derive(Clone, uniffi::Object)]
pub struct SessionContext {
    /// Wrapped core session context
    inner: crate::slimrpc_core::SessionContext,
}

#[uniffi::export]
impl SessionContext {
    /// Get the session ID
    pub fn session_id(&self) -> String {
        self.inner.session_id().to_string()
    }

    /// Get the source name (sender)
    pub fn source(&self) -> Arc<Name> {
        Arc::new(self.inner.source().clone().into())
    }

    /// Get the destination name (receiver)
    pub fn destination(&self) -> Arc<Name> {
        Arc::new(self.inner.destination().clone().into())
    }

    /// Get all session metadata as a map
    pub fn metadata(&self) -> HashMap<String, String> {
        self.inner.metadata().as_map().clone()
    }

    /// Get a specific metadata value
    pub fn get_metadata(&self, key: String) -> Option<String> {
        self.inner.metadata().get(&key).map(|s| s.to_string())
    }
}

/// Message-level context information
///
/// Contains information about individual messages in a stream including
/// source, destination, payload type, and message metadata.
#[derive(Clone, uniffi::Object)]
pub struct RpcMessageContext {
    /// Source name from message
    source: Name,
    /// Destination name from message
    destination: Option<Name>,
    /// Payload type
    payload_type: String,
    /// Message metadata
    metadata: HashMap<String, String>,
}

#[uniffi::export]
impl RpcMessageContext {
    /// Get the source name
    pub fn source(&self) -> Arc<Name> {
        Arc::new(self.source.clone())
    }

    /// Get the destination name (if available)
    pub fn destination(&self) -> Option<Arc<Name>> {
        self.destination.as_ref().map(|n| Arc::new(n.clone()))
    }

    /// Get the payload type
    pub fn payload_type(&self) -> String {
        self.payload_type.clone()
    }

    /// Get all message metadata as a map
    pub fn metadata(&self) -> HashMap<String, String> {
        self.metadata.clone()
    }

    /// Get a specific metadata value
    pub fn get_metadata(&self, key: String) -> Option<String> {
        self.metadata.get(&key).map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_conversion() {
        let name = Name::new_with_id(
            "org".to_string(),
            "namespace".to_string(),
            "service".to_string(),
            123,
        );

        let components = name.components();
        assert_eq!(components[0], "org");
        assert_eq!(components[1], "namespace");
        assert_eq!(components[2], "service");
        assert_eq!(name.id(), 123);
    }
}
