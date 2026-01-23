// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Test utilities for SlimRPC
//!
//! Provides common test message types and utilities for testing RPC functionality.

use bincode::{Decode, Encode};
use agntcy_slimrpc::{Decoder, Encoder, Status};

/// Simple request message for testing
#[derive(Debug, Clone, Default, PartialEq, Encode, Decode)]
pub struct TestRequest {
    pub message: String,
    pub value: i32,
}

impl Encoder for TestRequest {
    fn encode(&self) -> Result<Vec<u8>, Status> {
        let encoded = bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Status::internal(format!("Encoding error: {}", e)))?;
        Ok(encoded)
    }
}

impl Decoder for TestRequest {
    fn decode(buf: &[u8]) -> Result<Self, Status> {
        let (decoded, _len): (TestRequest, usize) =
            bincode::decode_from_slice(buf, bincode::config::standard())
                .map_err(|e| Status::invalid_argument(format!("Decoding error: {}", e)))?;

        Ok(decoded)
    }
}

/// Simple response message for testing
#[derive(Debug, Clone, Default, PartialEq, Encode, Decode)]
pub struct TestResponse {
    pub result: String,
    pub count: i32,
}

impl Encoder for TestResponse {
    fn encode(&self) -> Result<Vec<u8>, Status> {
        let encoded = bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Status::internal(format!("Encoding error: {}", e)))?;
        Ok(encoded)
    }
}

impl Decoder for TestResponse {
    fn decode(buf: &[u8]) -> Result<Self, Status> {
        let (decoded, _len): (TestResponse, usize) =
            bincode::decode_from_slice(buf, bincode::config::standard())
                .map_err(|e| Status::invalid_argument(format!("Decoding error: {}", e)))?;

        Ok(decoded)
    }
}
