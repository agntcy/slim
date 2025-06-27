// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum Identity {
    Jwt(String),
    Simple(String),
}

impl Identity {
    pub fn identifier(&self) -> &str {
        match self {
            Identity::Jwt(token) => token,
            Identity::Simple(name) => name,
        }
    }

    //TODO(zkacsand): find a better solution
    pub fn storage_identifier(&self) -> String {
        match self {
            Identity::Jwt(token) => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                token.hash(&mut hasher);
                format!("jwt_{:x}", hasher.finish())
            }
            Identity::Simple(name) => name.clone(),
        }
    }

    pub fn simple(name: impl Into<String>) -> Self {
        Identity::Simple(name.into())
    }

    pub fn jwt(token: impl Into<String>) -> Self {
        Identity::Jwt(token.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredIdentity {
    pub identifier: String,
    pub public_key_bytes: Vec<u8>,
    pub private_key_bytes: Vec<u8>,
}
