// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// CompressionType represents the supported compression types for gRPC messages.
/// The supported types are: Gzip, Zlib, Deflate, Snappy, Zstd, Lz4, None, and Empty.
/// The default type is None.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default, JsonSchema)]
pub enum CompressionType {
    Gzip,
    Zlib,
    Deflate,
    Snappy,
    Zstd,
    Lz4,
    #[default]
    None,
    Empty,
}

impl CompressionType {
    /// Determines if the compression type is considered "compressed"
    pub fn is_compressed(&self) -> bool {
        *self != CompressionType::None && *self != CompressionType::Empty
    }
}

/// Implement the FromStr trait to handle string conversion and parsing
impl FromStr for CompressionType {
    type Err = CompressionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gzip" => Ok(CompressionType::Gzip),
            "zlib" => Ok(CompressionType::Zlib),
            "deflate" => Ok(CompressionType::Deflate),
            "snappy" => Ok(CompressionType::Snappy),
            "zstd" => Ok(CompressionType::Zstd),
            "lz4" => Ok(CompressionType::Lz4),
            "none" => Ok(CompressionType::None),
            "" => Ok(CompressionType::Empty),
            _ => Err(CompressionError::UnsupportedType(s.to_string())),
        }
    }
}

/// Custom error type for handling unsupported compression types
#[derive(Error, Debug)]
pub enum CompressionError {
    // Parsing / unsupported compression type
    #[error("unsupported compression type {0}")]
    UnsupportedType(String),
}
