// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MlsError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("Serialization/Deserialization error: {0}")]
    Serde(String),
    #[error("MLS error: {0}")]
    Mls(String),
    #[error("Requested ciphersuite is unavailable")]
    CiphersuiteUnavailable,
}
