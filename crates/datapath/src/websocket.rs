// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! WebSocket transport tasks for the data plane.
//!
//! Two implementations expose an identical surface
//! ([`spawn_transport_tasks`] producing [`WebSocketStreams`]) so the rest of
//! the data plane is agnostic to which one is compiled in:
//!
//! * native (`stream`) — server + client over `fastwebsockets`.
//! * browser (`stream_wasm`) — client over `gloo_net`'s JS `WebSocket`, for
//!   `wasm32-unknown-unknown` builds.

#[cfg(not(target_arch = "wasm32"))]
mod stream;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use stream::spawn_transport_tasks;

#[cfg(target_arch = "wasm32")]
mod stream_wasm;
#[cfg(target_arch = "wasm32")]
pub use stream_wasm::{WebSocketStreams, spawn_transport_tasks};
