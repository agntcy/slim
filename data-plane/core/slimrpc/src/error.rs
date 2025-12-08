// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SRPCError {
    #[error("RPC response error: code={0}, message={1}")]
    ResponseError(i32, String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("No handler found for: {0}")]
    NoHandler(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, SRPCError>;
