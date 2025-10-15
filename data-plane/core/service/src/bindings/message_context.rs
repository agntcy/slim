// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use slim_datapath::api::{ProtoMessage, ProtoPublishType};
use slim_datapath::messages::Name;

use crate::errors::ServiceError;

/// Generic message context for language bindings
///
/// Provides routing and descriptive metadata needed for replying,
/// auditing, and instrumentation across different language bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageContext {
    /// Fully-qualified sender identity
    pub source_name: Name,
    /// Fully-qualified destination identity (may be empty for broadcast/group scenarios)
    pub destination_name: Option<Name>,
    /// Logical/semantic type (defaults to "msg" if unspecified)
    pub payload_type: String,
    /// Arbitrary key/value pairs supplied by the sender (e.g. tracing IDs)
    pub metadata: HashMap<String, String>,
    /// Numeric identifier of the inbound connection carrying the message
    pub input_connection: u64,
}

impl MessageContext {
    /// Create a new MessageContext
    pub fn new(
        source: Name,
        destination: Option<Name>,
        payload_type: String,
        metadata: HashMap<String, String>,
        input_connection: u64,
    ) -> Self {
        Self {
            source_name: source,
            destination_name: destination,
            payload_type,
            metadata,
            input_connection,
        }
    }

    /// Build a `MessageContext` plus the raw payload bytes from a low-level
    /// `ProtoMessage`. Returns an error if the message type is unsupported
    /// (i.e. not a publish payload).
    ///
    /// On success:
    /// * The context captures source/destination identities
    /// * `payload_type` defaults to "msg" if unset
    /// * `metadata` is copied from the underlying protocol envelope
    /// * The returned `Vec<u8>` is the raw application payload
    pub fn from_proto_message(msg: ProtoMessage) -> Result<(Self, Vec<u8>), ServiceError> {
        let Some(ProtoPublishType(publish)) = msg.message_type.as_ref() else {
            return Err(ServiceError::ReceiveError(
                "unsupported message type".to_string(),
            ));
        };

        let source = msg.get_source();
        let destination = Some(msg.get_dst());
        let input_connection = msg.get_incoming_conn();
        let payload_bytes = publish
            .msg
            .as_ref()
            .map(|c| c.blob.clone())
            .unwrap_or_default();
        let payload_type = publish
            .msg
            .as_ref()
            .map(|c| c.content_type.clone())
            .unwrap_or_else(|| "msg".to_string());
        let metadata = msg.get_metadata_map();

        let ctx = Self::new(
            source,
            destination,
            payload_type,
            metadata,
            input_connection,
        );
        Ok((ctx, payload_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::messages::Name;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_message_context_creation() {
        let source = Name::from_strings(["org", "namespace", "sender"]);
        let destination = Some(Name::from_strings(["org", "namespace", "receiver"]));
        let mut metadata = HashMap::new();
        metadata.insert("test".to_string(), "value".to_string());

        let ctx = MessageContext::new(
            source.clone(),
            destination.clone(),
            "application/json".to_string(),
            metadata.clone(),
            42,
        );

        assert_eq!(ctx.source_name, source);
        assert_eq!(ctx.destination_name, destination);
        assert_eq!(ctx.payload_type, "application/json");
        assert_eq!(ctx.metadata, metadata);
        assert_eq!(ctx.input_connection, 42);
    }
}
