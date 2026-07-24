// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs_core::error::IntoAnyError;
use thiserror::Error;

/// Errors from the encrypted persistence layer.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("storage backend error: {0}")]
    Storage(String),
}

// Required so `PersistenceError` can be the associated `Error` of the
// `GroupStateStorage` trait.
impl IntoAnyError for PersistenceError {}
