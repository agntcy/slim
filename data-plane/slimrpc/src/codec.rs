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
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status>;

    /// Encode a message to a new Vec<u8>
    fn encode_to_vec(&self) -> Result<Vec<u8>, Status> {
        let mut buf = Vec::new();
        self.encode(&mut buf)?;
        Ok(buf)
    }

    /// Get the encoded size of the message
    fn encoded_len(&self) -> usize;
}

/// Trait for decoding messages from bytes
pub trait Decoder: Default {
    /// Decode a message from bytes
    fn decode(buf: &[u8]) -> Result<Self, Status>;

    /// Merge encoded data into this message
    fn merge(&mut self, buf: &[u8]) -> Result<(), Status>;
}

/// Combined codec trait for types that can be both encoded and decoded
pub trait Codec: Encoder + Decoder {}

// Blanket implementation
impl<T: Encoder + Decoder> Codec for T {}

/// Helper function to encode a message
pub fn encode<T: Encoder>(message: &T) -> Result<Vec<u8>, Status> {
    message.encode_to_vec()
}

/// Helper function to decode a message
pub fn decode<T: Decoder>(buf: &[u8]) -> Result<T, Status> {
    T::decode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock message type for testing
    #[derive(Debug, Clone, Default, PartialEq)]
    struct TestMessage {
        data: Vec<u8>,
    }

    impl Encoder for TestMessage {
        fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status> {
            buf.extend_from_slice(&self.data);
            Ok(())
        }

        fn encoded_len(&self) -> usize {
            self.data.len()
        }
    }

    impl Decoder for TestMessage {
        fn decode(buf: &[u8]) -> Result<Self, Status> {
            Ok(TestMessage {
                data: buf.to_vec(),
            })
        }

        fn merge(&mut self, buf: &[u8]) -> Result<(), Status> {
            self.data.extend_from_slice(buf);
            Ok(())
        }
    }

    #[test]
    fn test_encode() {
        let msg = TestMessage {
            data: vec![1, 2, 3, 4],
        };
        let encoded = encode(&msg).unwrap();
        assert_eq!(encoded, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_decode() {
        let buf = vec![1, 2, 3, 4];
        let msg: TestMessage = decode(&buf).unwrap();
        assert_eq!(msg.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_encoded_len() {
        let msg = TestMessage {
            data: vec![1, 2, 3, 4],
        };
        assert_eq!(msg.encoded_len(), 4);
    }

    #[test]
    fn test_merge() {
        let mut msg = TestMessage {
            data: vec![1, 2],
        };
        msg.merge(&[3, 4]).unwrap();
        assert_eq!(msg.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_codec_trait() {
        fn process<T: Codec>(_t: &T) {}
        let msg = TestMessage::default();
        process(&msg); // Should compile due to blanket impl
    }
}