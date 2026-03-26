// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use once_cell::sync::Lazy;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
pub static INSTANCE_ID: Lazy<String> =
    Lazy::new(|| std::env::var("SLIM_INSTANCE_ID").unwrap_or_else(|_| Uuid::new_v4().to_string()));

#[cfg(target_arch = "wasm32")]
pub static INSTANCE_ID: Lazy<String> =
    Lazy::new(|| Uuid::new_v4().to_string());
