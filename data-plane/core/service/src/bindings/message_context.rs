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
    /// Identity contained in the message
    pub identity: String,
}

impl MessageContext {
    /// Create a new MessageContext
    pub fn new(
        source: Name,
        destination: Option<Name>,
        payload_type: String,
        metadata: HashMap<String, String>,
        input_connection: u64,
        identity: String,
    ) -> Self {
        Self {
            source_name: source,
            destination_name: destination,
            payload_type,
            metadata,
            input_connection,
            identity,
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
            .and_then(|c| c.as_application_payload().ok())
            .map(|p| p.blob.clone())
            .unwrap_or_default();
        let payload_type = publish
            .msg
            .as_ref()
            .and_then(|c| c.as_application_payload().ok())
            .map(|p| {
                if p.payload_type.is_empty() {
                    "msg".to_string()
                } else {
                    p.payload_type.clone()
                }
            })
            .unwrap_or_else(|| "msg".to_string());
        let metadata = msg.get_metadata_map();
        let identity = msg.get_identity();

        let ctx = Self::new(
            source,
            destination,
            payload_type,
            metadata,
            input_connection,
            identity,
        );
        Ok((ctx, payload_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::api::{
        ApplicationPayload, ProtoMessage, ProtoPublish, ProtoPublishType, SessionHeader, SlimHeader,
    };
    use slim_datapath::messages::Name;
    use std::collections::HashMap;

    // Helper function to create a test ProtoMessage with Publish type
    fn create_test_proto_message(
        source: Name,
        dest: Name,
        connection_id: u64,
        payload: Vec<u8>,
        content_type: String,
        metadata: HashMap<String, String>,
    ) -> ProtoMessage {
        let content = ApplicationPayload::new(&content_type, payload).as_content();

        let mut slim_header = SlimHeader::default();
        slim_header.set_source(&source);
        slim_header.set_destination(&dest);

        let publish = ProtoPublish {
            header: Some(slim_header),
            session: Some(SessionHeader::default()),
            msg: Some(content),
        };

        let mut proto_msg = ProtoMessage {
            message_type: Some(ProtoPublishType(publish)),
            metadata,
        };

        proto_msg.set_incoming_conn(Some(connection_id));

        proto_msg
    }

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
            "test-identity".to_string(),
        );

        assert_eq!(ctx.source_name, source);
        assert_eq!(ctx.destination_name, destination);
        assert_eq!(ctx.payload_type, "application/json");
        assert_eq!(ctx.metadata, metadata);
        assert_eq!(ctx.input_connection, 42);
        assert_eq!(ctx.identity, "test-identity");
    }

    #[tokio::test]
    async fn test_from_proto_message_success() {
        // Create test data
        let payload_data = b"test payload".to_vec();
        let content_type = "application/json".to_string();
        let source_name = Name::from_strings(["org", "sender", "service"]);
        let dest_name = Name::from_strings(["org", "receiver", "service"]);
        let connection_id = 12345u64;
        let identity = "test-identity".to_string();

        // Create metadata
        let mut metadata = HashMap::new();
        metadata.insert("trace_id".to_string(), "abc123".to_string());
        metadata.insert("user_id".to_string(), "user456".to_string());

        let mut proto_msg = create_test_proto_message(
            source_name.clone(),
            dest_name.clone(),
            connection_id,
            payload_data.clone(),
            content_type.clone(),
            metadata.clone(),
        );

        // set identity
        proto_msg
            .get_slim_header_mut()
            .set_identity(identity.clone());
        // Test from_proto_message
        let result = MessageContext::from_proto_message(proto_msg);
        assert!(result.is_ok());

        let (ctx, payload) = result.unwrap();

        // Verify context fields
        assert_eq!(ctx.source_name, source_name);
        assert_eq!(ctx.destination_name, Some(dest_name));
        assert_eq!(ctx.payload_type, content_type);
        assert_eq!(ctx.metadata, metadata);
        assert_eq!(ctx.input_connection, connection_id);
        assert_eq!(ctx.identity, identity);

        // Verify payload
        assert_eq!(payload, payload_data);
    }

    #[tokio::test]
    async fn test_from_proto_message_with_default_content_type() {
        let source_name = Name::from_strings(["org", "sender", "service"]);
        let dest_name = Name::from_strings(["org", "receiver", "service"]);
        let payload_data = b"test payload".to_vec();

        let proto_msg = create_test_proto_message(
            source_name,
            dest_name,
            42,
            payload_data,
            String::new(), // Empty content type should default to "msg"
            HashMap::new(),
        );

        let result = MessageContext::from_proto_message(proto_msg);
        assert!(result.is_ok());

        let (ctx, _) = result.unwrap();
        assert_eq!(ctx.payload_type, "msg"); // Should default to "msg"
    }

    #[tokio::test]
    async fn test_from_proto_message_with_no_content() {
        let source_name = Name::from_strings(["org", "sender", "service"]);
        let _dest_name = Name::from_strings(["org", "receiver", "service"]);

        // Create ProtoPublish without msg content
        let mut slim_header = SlimHeader::default();
        slim_header.set_source(&source_name);
        slim_header.set_destination(&_dest_name);

        let publish = ProtoPublish {
            header: Some(slim_header),
            session: Some(SessionHeader::default()),
            msg: None, // No content
        };

        let mut proto_msg = ProtoMessage {
            message_type: Some(ProtoPublishType(publish)),
            ..Default::default()
        };
        proto_msg.set_incoming_conn(Some(42));

        let result = MessageContext::from_proto_message(proto_msg);
        assert!(result.is_ok());

        let (ctx, payload) = result.unwrap();
        assert_eq!(ctx.payload_type, "msg"); // Should default to "msg"
        assert_eq!(payload, Vec::<u8>::new()); // Should be empty payload
    }

    #[tokio::test]
    async fn test_from_proto_message_unsupported_message_type() {
        // Create ProtoMessage without ProtoPublishType
        let proto_msg = ProtoMessage {
            message_type: None, // Unsupported type
            ..Default::default()
        };

        let result = MessageContext::from_proto_message(proto_msg);
        assert!(result.is_err());

        match result.unwrap_err() {
            ServiceError::ReceiveError(msg) => {
                assert_eq!(msg, "unsupported message type");
            }
            _ => panic!("Expected ReceiveError"),
        }
    }

    #[tokio::test]
    async fn test_from_proto_message_with_empty_metadata() {
        let source_name = Name::from_strings(["test", "source", "v1"]);
        let dest_name = Name::from_strings(["test", "dest", "v1"]);
        let payload_data = b"test".to_vec();

        let proto_msg = create_test_proto_message(
            source_name.clone(),
            dest_name.clone(),
            99,
            payload_data.clone(),
            "text/plain".to_string(),
            HashMap::new(), // Empty metadata
        );

        let result = MessageContext::from_proto_message(proto_msg);
        assert!(result.is_ok());

        let (ctx, payload) = result.unwrap();
        assert_eq!(ctx.source_name, source_name);
        assert_eq!(ctx.destination_name, Some(dest_name));
        assert_eq!(ctx.payload_type, "text/plain");
        assert_eq!(ctx.metadata, HashMap::new()); // Should be empty
        assert_eq!(ctx.input_connection, 99);
        assert_eq!(payload, payload_data);
    }
}
