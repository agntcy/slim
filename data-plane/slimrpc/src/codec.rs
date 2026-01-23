// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Codec traits for message serialization and deserialization
//!
//! Provides trait abstractions for encoding and decoding messages in SlimRPC.
//! These traits are typically implemented by protobuf-generated code.

use crate::Status;

/// Trait for encoding messages to bytes
pub trait Encoder {
    /// Encode a message to bytes
    fn encode(&self) -> Result<Vec<u8>, Status>;
}

/// Trait for decoding messages from bytes
pub trait Decoder: Default {
    /// Decode a message from bytes
    fn decode(buf: &[u8]) -> Result<Self, Status>;
}

/// Combined codec trait for types that can be both encoded and decoded
pub trait Codec: Encoder + Decoder {}

// Blanket implementation
impl<T: Encoder + Decoder> Codec for T {}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple test message type for codec tests
    #[derive(Debug, Clone, Default, PartialEq)]
    struct TestMessage {
        data: Vec<u8>,
    }

    impl Encoder for TestMessage {
        fn encode(&self) -> Result<Vec<u8>, Status> {
            Ok(self.data.clone())
        }
    }

    impl Decoder for TestMessage {
        fn decode(buf: &[u8]) -> Result<Self, Status> {
            Ok(TestMessage {
                data: buf.to_vec(),
            })
        }
    }

    #[test]
    fn test_encode() {
        let msg = TestMessage {
            data: vec![1, 2, 3, 4],
        };
        let encoded = msg.encode().unwrap();
        assert_eq!(encoded, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_decode() {
        let buf = vec![1, 2, 3, 4];
        let msg: TestMessage = TestMessage::decode(&buf).unwrap();
        assert_eq!(msg.data, vec![1, 2, 3, 4]);
    }
}
