// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Context types for SlimRPC request handling
//!
//! Provides context information for RPC handlers including metadata, deadlines,
//! session information, and message routing details.

use std::time::{Duration, SystemTime};

use slim_datapath::messages::Name;
use slim_session::context::SessionContext as SlimSessionContext;

use super::{DEADLINE_KEY, Metadata};

/// Context passed to RPC handlers
///
/// Contains all contextual information about an RPC call including:
/// - Session information (source, destination, session ID)
/// - Metadata (key-value pairs)
/// - Deadline/timeout information
/// - Message routing details
#[derive(Debug, Clone)]
pub struct Context {
    /// Session context information
    session: SessionContext,
    /// Message context information (for individual messages in streams)
    message: Option<MessageContext>,
    /// Request metadata
    metadata: Metadata,
    /// Deadline for the RPC call
    deadline: Option<SystemTime>,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Create a new empty context
    pub fn new() -> Self {
        Self {
            session: SessionContext {
                session_id: String::new(),
                source: Name::from_strings(["", "", ""]),
                destination: Name::from_strings(["", "", ""]),
                metadata: Metadata::new(),
            },
            message: None,
            metadata: Metadata::new(),
            deadline: None,
        }
    }

    /// Create a new context from a session
    pub fn from_session(session: &SlimSessionContext) -> Self {
        // Get metadata from session controller
        let session_arc = session.session_arc();
        let metadata = if let Some(controller) = session_arc {
            Metadata::from_map(controller.metadata())
        } else {
            Metadata::new()
        };
        let deadline = Self::parse_deadline(&metadata);

        Self {
            session: SessionContext::from_session(session),
            message: None,
            metadata,
            deadline,
        }
    }

    /// Create a new context from a Session wrapper
    pub async fn from_session_wrapper(session: &super::Session) -> Self {
        let session_id = session.session_id().await.to_string();
        let source = session.source().await;
        let destination = session.destination().await;
        let metadata_map = session.metadata().await;
        let metadata = Metadata::from_map(metadata_map);
        let deadline = Self::parse_deadline(&metadata);

        Self {
            session: SessionContext {
                session_id,
                source,
                destination,
                metadata: metadata.clone(),
            },
            message: None,
            metadata,
            deadline,
        }
    }

    /// Create a new context with message metadata
    pub fn with_message_metadata(
        mut self,
        msg_metadata: std::collections::HashMap<String, String>,
    ) -> Self {
        let msg_meta = Metadata::from_map(msg_metadata);
        // Merge message metadata into context metadata
        self.metadata.merge(msg_meta);
        // Re-parse deadline from merged metadata (message metadata takes precedence)
        self.deadline = Self::parse_deadline(&self.metadata);
        self
    }

    /// Get the session context
    pub fn session(&self) -> &SessionContext {
        &self.session
    }

    /// Get the message context (if available)
    pub fn message(&self) -> Option<&MessageContext> {
        self.message.as_ref()
    }

    /// Get the request metadata
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Get a mutable reference to metadata
    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

    /// Get the deadline for this RPC call
    pub fn deadline(&self) -> Option<SystemTime> {
        self.deadline
    }

    /// Get the remaining time until deadline
    pub fn remaining_time(&self) -> Option<Duration> {
        self.deadline.and_then(|deadline| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .and_then(|now| {
                    deadline
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| d.checked_sub(now))
                })
        })
    }

    /// Check if the deadline has been exceeded
    pub fn is_deadline_exceeded(&self) -> bool {
        if let Some(deadline) = self.deadline {
            SystemTime::now() > deadline
        } else {
            false
        }
    }

    /// Parse deadline from metadata
    fn parse_deadline(metadata: &Metadata) -> Option<SystemTime> {
        metadata.get(DEADLINE_KEY).and_then(|deadline_str| {
            deadline_str.parse::<f64>().ok().and_then(|seconds| {
                SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs_f64(seconds))
            })
        })
    }

    /// Set the deadline
    pub fn set_deadline(&mut self, deadline: SystemTime) {
        self.deadline = Some(deadline);
        let seconds = deadline
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.metadata.insert(DEADLINE_KEY, seconds.to_string());
    }

    /// Set the deadline from a duration
    pub fn set_timeout(&mut self, timeout: Duration) {
        let deadline = SystemTime::now()
            .checked_add(timeout)
            .unwrap_or(SystemTime::now());
        self.set_deadline(deadline);
    }
}

/// Session-level context information
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Session ID
    session_id: String,
    /// Source name (sender)
    source: Name,
    /// Destination name (receiver)
    destination: Name,
    /// Session metadata
    metadata: Metadata,
}

impl SessionContext {
    /// Create from a SLIM session context
    pub fn from_session(session: &SlimSessionContext) -> Self {
        let session_arc = session.session_arc();
        if let Some(controller) = session_arc {
            Self {
                session_id: controller.id().to_string(),
                source: controller.source().clone(),
                destination: controller.dst().clone(),
                metadata: Metadata::from_map(controller.metadata()),
            }
        } else {
            // Fallback if session is already closed
            Self {
                session_id: session.session_id().to_string(),
                source: Name::from_strings(["", "", ""]),
                destination: Name::from_strings(["", "", ""]),
                metadata: Metadata::new(),
            }
        }
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the source name
    pub fn source(&self) -> &Name {
        &self.source
    }

    /// Get the destination name
    pub fn destination(&self) -> &Name {
        &self.destination
    }

    /// Get the session metadata
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

/// Message-level context information
#[derive(Debug, Clone)]
pub struct MessageContext {
    /// Source name from message
    source: Name,
    /// Destination name from message
    destination: Option<Name>,
    /// Payload type
    payload_type: String,
    /// Message metadata
    metadata: Metadata,
}

impl MessageContext {
    /// Create a new message context
    pub fn new(
        source: Name,
        destination: Option<Name>,
        payload_type: String,
        metadata: Metadata,
    ) -> Self {
        Self {
            source,
            destination,
            payload_type,
            metadata,
        }
    }

    /// Get the source name
    pub fn source(&self) -> &Name {
        &self.source
    }

    /// Get the destination name
    pub fn destination(&self) -> Option<&Name> {
        self.destination.as_ref()
    }

    /// Get the payload type
    pub fn payload_type(&self) -> &str {
        &self.payload_type
    }

    /// Get the message metadata
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_context_deadline_parsing() {
        let mut metadata = Metadata::new();
        let deadline_time = SystemTime::now()
            .checked_add(Duration::from_secs(60))
            .unwrap();
        let seconds = deadline_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        metadata.insert(DEADLINE_KEY, seconds.to_string());

        let parsed = Context::parse_deadline(&metadata);
        assert!(parsed.is_some());
    }

    #[test]
    fn test_context_deadline_exceeded() {
        let mut metadata = Metadata::new();
        let past_deadline = SystemTime::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap();
        let seconds = past_deadline
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        metadata.insert(DEADLINE_KEY, seconds.to_string());

        let deadline = Context::parse_deadline(&metadata);
        assert!(deadline.is_some());
        if let Some(d) = deadline {
            assert!(SystemTime::now() > d);
        }
    }

    #[test]
    fn test_context_set_timeout() {
        let session_metadata = HashMap::new();
        let ctx_session = SessionContext {
            session_id: "test-session".to_string(),
            source: Name::from_strings(["org", "ns", "app"]),
            destination: Name::from_strings(["org", "ns", "dest"]),
            metadata: Metadata::from_map(session_metadata),
        };

        let mut ctx = Context {
            session: ctx_session,
            message: None,
            metadata: Metadata::new(),
            deadline: None,
        };

        ctx.set_timeout(Duration::from_secs(30));
        assert!(ctx.deadline().is_some());
        assert!(!ctx.is_deadline_exceeded());
    }

    #[test]
    fn test_remaining_time() {
        let session_metadata = HashMap::new();
        let ctx_session = SessionContext {
            session_id: "test-session".to_string(),
            source: Name::from_strings(["org", "ns", "app"]),
            destination: Name::from_strings(["org", "ns", "dest"]),
            metadata: Metadata::from_map(session_metadata),
        };

        let mut ctx = Context {
            session: ctx_session,
            message: None,
            metadata: Metadata::new(),
            deadline: None,
        };

        ctx.set_timeout(Duration::from_secs(60));
        let remaining = ctx.remaining_time();
        assert!(remaining.is_some());
        if let Some(r) = remaining {
            // Should be close to 60 seconds, allow some margin
            assert!(r.as_secs() >= 59 && r.as_secs() <= 60);
        }
    }
}
