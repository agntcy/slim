// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod utils;

#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
pub use native::*;

#[cfg(all(feature = "wasm", not(feature = "native")))]
mod wasm;
#[cfg(all(feature = "wasm", not(feature = "native")))]
pub use wasm::*;
