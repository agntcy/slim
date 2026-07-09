// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Observability for the SLIM data plane.
//!
//! On **native** targets this provides the full OpenTelemetry + tracing-subscriber
//! pipeline (logs, distributed traces, metrics).
//!
//! On **wasm32** targets a lightweight subscriber pipes tracing events to the
//! browser console (`console.log` / `console.warn` / `console.error`). With the
//! `otel_tracing` feature, W3C trace-context propagation is enabled for
//! cross-service continuity (no OTLP export).

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::*;
