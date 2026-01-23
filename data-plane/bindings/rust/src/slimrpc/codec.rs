// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Codec traits for UniFFI
//!
//! Provides trait definitions for message encoding/decoding.
//! Note: Trait definitions cannot be directly exposed through UniFFI,
//! so these are primarily for Rust-side use.

use super::error::Status;

/// Trait for encoding messages to bytes
///
/// This trait should be implemented by message types that need to be
/// serialized for RPC transmission.
pub trait Encoder {
    /// Encode a message to bytes
    fn encode(&self) -> Result<Vec<u8>, Status>;
}

/// Trait for decoding messages from bytes
///
/// This trait should be implemented by message types that need to be
/// deserialized from RPC transmission.
pub trait Decoder: Default {
    /// Decode a message from bytes
    fn decode(buf: &[u8]) -> Result<Self, Status>;
}

/// Combined codec trait for types that can be both encoded and decoded
pub trait Codec: Encoder + Decoder {}

// Blanket implementation
impl<T: Encoder + Decoder> Codec for T {}

// Standard implementations for common types
impl Encoder for Vec<u8> {
    fn encode(&self) -> Result<Vec<u8>, Status> {
        Ok(self.clone())
    }
}

impl Decoder for Vec<u8> {
    fn decode(buf: &[u8]) -> Result<Self, Status> {
        Ok(buf.to_vec())
    }
}

impl Encoder for String {
    fn encode(&self) -> Result<Vec<u8>, Status> {
        Ok(self.as_bytes().to_vec())
    }
}

impl Decoder for String {
    fn decode(buf: &[u8]) -> Result<Self, Status> {
        String::from_utf8(buf.to_vec()).map_err(|e| {
            Status::new(
                super::error::Code::InvalidArgument,
                format!("Failed to decode string: {}", e),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vec_u8_codec() {
        let data = vec![1, 2, 3, 4, 5];
        let encoded = data.encode().unwrap();
        assert_eq!(encoded, data);

        let decoded = Vec::<u8>::decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_string_codec() {
        let data = "Hello, World!".to_string();
        let encoded = data.encode().unwrap();
        assert_eq!(encoded, data.as_bytes());

        let decoded = String::decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_string_decode_invalid_utf8() {
        let invalid_utf8 = vec![0xFF, 0xFE, 0xFD];
        let result = String::decode(&invalid_utf8);
        assert!(result.is_err());
    }
}