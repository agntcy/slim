// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "pyo3")]
use pyo3::prelude::*;

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// CompressionType represents the supported compression types for gRPC messages.
/// The supported types are: Gzip, Zlib, Deflate, Snappy, Zstd, Lz4, None, and Empty.
/// The default type is None.
#[derive(Debug, Deserialize, PartialEq, Clone, Default)]
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

impl std::fmt::Display for CompressionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompressionType::Gzip => write!(f, "gzip"),
            CompressionType::Zlib => write!(f, "zlib"),
            CompressionType::Deflate => write!(f, "deflate"),
            CompressionType::Snappy => write!(f, "snappy"),
            CompressionType::Zstd => write!(f, "zstd"),
            CompressionType::Lz4 => write!(f, "lz4"),
            CompressionType::None => write!(f, "none"),
            CompressionType::Empty => write!(f, ""),
        }
    }
}

#[cfg(feature = "pyo3")]
impl<'py> IntoPyObject<'py> for CompressionType {
    type Target = pyo3::types::PyString;
    type Output = Bound<'py, Self::Target>; // in most cases this will be `Bound`
    type Error = PyErr; // the conversion error type, has to be convertable to `PyErr`

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let py_str = pyo3::types::PyString::new(py, &self.to_string());
        Ok(py_str.into())
    }
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
#[derive(Debug)]
pub enum CompressionError {
    UnsupportedType(String),
}

/// Implement the Display trait for better error messages.
impl fmt::Display for CompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompressionError::UnsupportedType(t) => {
                write!(f, "unsupported compression type {:?}", t)
            }
        }
    }
}
