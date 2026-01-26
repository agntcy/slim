// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Context types for UniFFI
//!
//! Provides UniFFI-compatible wrappers around the core SlimRPC context types.

use std::sync::Arc;
use std::time::SystemTime;

#[cfg(test)]
use std::time::Duration;

use crate::Name;

use super::metadata::Metadata;

/// Context passed to RPC handlers (UniFFI-compatible)
///
/// Contains all contextual information about an RPC call including:
/// - Session information (source, destination, session ID)
/// - Metadata (key-value pairs)
/// - Deadline/timeout information
#[derive(Debug, Clone, uniffi::Object)]
pub struct RpcContext {
    /// Session context information
    session: Arc<RpcSessionContext>,
    /// Request metadata
    metadata: Arc<Metadata>,
    /// Deadline for the RPC call (seconds since UNIX epoch)
    deadline_secs: Option<f64>,
}

#[uniffi::export]
impl RpcContext {
    /// Get the session context
    pub fn session(&self) -> Arc<RpcSessionContext> {
        self.session.clone()
    }

    /// Get the request metadata
    pub fn metadata(&self) -> Arc<Metadata> {
        self.metadata.clone()
    }

    /// Get the deadline as seconds since UNIX epoch
    pub fn deadline_secs(&self) -> Option<f64> {
        self.deadline_secs
    }

    /// Get the remaining time until deadline in seconds
    pub fn remaining_time_secs(&self) -> Option<f64> {
        self.deadline_secs.and_then(|deadline_secs| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()?
                .as_secs_f64();
            let remaining = deadline_secs - now;
            if remaining > 0.0 {
                Some(remaining)
            } else {
                Some(0.0)
            }
        })
    }

    /// Check if the deadline has been exceeded
    pub fn is_deadline_exceeded(&self) -> bool {
        if let Some(remaining) = self.remaining_time_secs() {
            remaining <= 0.0
        } else {
            false
        }
    }
}

// Internal conversion methods
impl RpcContext {
    /// Create from core context
    pub(crate) fn from_core(context: agntcy_slimrpc::Context) -> Self {
        let session = RpcSessionContext::from_core(context.session());
        let metadata = Metadata::from_core(context.metadata().clone());
        let deadline_secs = context.deadline().and_then(|d| {
            d.duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|dur| dur.as_secs_f64())
        });

        Self {
            session: Arc::new(session),
            metadata: Arc::new(metadata),
            deadline_secs,
        }
    }

    /// Create from core context (alias for from_core)
    pub(crate) fn from_core_context(context: agntcy_slimrpc::Context) -> Self {
        Self::from_core(context)
    }

    /// Convert to core context (approximation - some information may be lost)
    pub(crate) fn to_core(&self) -> agntcy_slimrpc::Context {
        // Note: This is a lossy conversion as we can't fully reconstruct the core context
        // from the FFI-friendly representation. This should primarily be used for reading.
        unimplemented!("Context to_core conversion not fully implemented - use from_core instead")
    }
}

/// Session-level context information (UniFFI-compatible)
#[derive(Debug, Clone, uniffi::Object)]
pub struct RpcSessionContext {
    /// Session ID
    session_id: String,
    /// Source name (sender)
    source: Arc<Name>,
    /// Destination name (receiver)
    destination: Arc<Name>,
    /// Session metadata as HashMap
    metadata: Arc<Metadata>,
}

#[uniffi::export]
impl RpcSessionContext {
    /// Get the session ID
    pub fn session_id(&self) -> String {
        self.session_id.clone()
    }

    /// Get the source name
    pub fn source(&self) -> Arc<Name> {
        self.source.clone()
    }

    /// Get the destination name
    pub fn destination(&self) -> Arc<Name> {
        self.destination.clone()
    }

    /// Get the session metadata
    pub fn metadata(&self) -> Arc<Metadata> {
        self.metadata.clone()
    }
}

impl RpcSessionContext {
    /// Create from core session context
    pub(crate) fn from_core(context: &agntcy_slimrpc::SessionContext) -> Self {
        Self {
            session_id: context.session_id().to_string(),
            source: Arc::new(Name::from_slim_name(context.source().clone())),
            destination: Arc::new(Name::from_slim_name(context.destination().clone())),
            metadata: Arc::new(Metadata::from_core(context.metadata().clone())),
        }
    }
}

/// Message-level context information (UniFFI-compatible)
#[derive(Debug, Clone, uniffi::Object)]
pub struct RpcMessageContext {
    /// Source name from message
    source: Arc<Name>,
    /// Destination name from message
    destination: Option<Arc<Name>>,
    /// Payload type
    payload_type: String,
    /// Message metadata
    metadata: Arc<Metadata>,
}

#[uniffi::export]
impl RpcMessageContext {
    /// Get the source name
    pub fn source(&self) -> Arc<Name> {
        self.source.clone()
    }

    /// Get the destination name
    pub fn destination(&self) -> Option<Arc<Name>> {
        self.destination.clone()
    }

    /// Get the payload type
    pub fn payload_type(&self) -> String {
        self.payload_type.clone()
    }

    /// Get the message metadata
    pub fn metadata(&self) -> Arc<Metadata> {
        self.metadata.clone()
    }
}

impl RpcMessageContext {
    /// Create from core message context
    pub(crate) fn from_core(context: &agntcy_slimrpc::MessageContext) -> Self {
        Self {
            source: Arc::new(Name::from_slim_name(context.source().clone())),
            destination: context
                .destination()
                .map(|d| Arc::new(Name::from_slim_name(d.clone()))),
            payload_type: context.payload_type().to_string(),
            metadata: Arc::new(Metadata::from_core(context.metadata().clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_session_context_creation() {
        let ctx = RpcSessionContext {
            session_id: "test-session".to_string(),
            source: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "client".to_string(),
            )),
            destination: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "server".to_string(),
            )),
            metadata: Metadata::new(),
        };

        assert_eq!(ctx.session_id, "test-session");
        assert_eq!(ctx.source.components().len(), 3);
        assert_eq!(ctx.destination.components().len(), 3);
    }

    #[test]
    fn test_rpc_message_context_creation() {
        let ctx = RpcMessageContext {
            source: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "client".to_string(),
            )),
            destination: Some(Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "server".to_string(),
            ))),
            payload_type: "msg".to_string(),
            metadata: Metadata::new(),
        };

        assert_eq!(ctx.payload_type, "msg");
        assert!(ctx.destination.is_some());
    }

    #[test]
    fn test_rpc_context_deadline() {
        let session = RpcSessionContext {
            session_id: "test-session".to_string(),
            source: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "client".to_string(),
            )),
            destination: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "server".to_string(),
            )),
            metadata: Metadata::new(),
        };

        let future_time = SystemTime::now() + Duration::from_secs(60);
        let deadline_secs = future_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let ctx = RpcContext {
            session: Arc::new(session),
            metadata: Metadata::new(),
            deadline_secs: Some(deadline_secs),
        };

        assert!(ctx.deadline_secs().is_some());
        assert!(!ctx.is_deadline_exceeded());

        let remaining = ctx.remaining_time_secs();
        assert!(remaining.is_some());
        if let Some(r) = remaining {
            assert!(r > 0.0 && r <= 60.0);
        }
    }

    #[test]
    fn test_rpc_context_deadline_exceeded() {
        let session = RpcSessionContext {
            session_id: "test-session".to_string(),
            source: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "client".to_string(),
            )),
            destination: Arc::new(Name::new(
                "org".to_string(),
                "ns".to_string(),
                "server".to_string(),
            )),
            metadata: Metadata::new(),
        };

        let past_time = SystemTime::now() - Duration::from_secs(60);
        let deadline_secs = past_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let ctx = RpcContext {
            session: Arc::new(session),
            metadata: Metadata::new(),
            deadline_secs: Some(deadline_secs),
        };

        assert!(ctx.is_deadline_exceeded());
        assert_eq!(ctx.remaining_time_secs(), Some(0.0));
    }
}
