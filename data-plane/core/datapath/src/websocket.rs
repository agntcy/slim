// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "native")]
pub(crate) mod stream;

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub(crate) mod stream_wasm;

#[cfg(feature = "native")]
pub(crate) use stream::spawn_transport_tasks;

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub(crate) use stream_wasm::spawn_transport_tasks;
