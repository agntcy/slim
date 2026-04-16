// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

pub static INSTANCE_ID: Lazy<String> =
    Lazy::new(|| std::env::var("SLIM_INSTANCE_ID").unwrap_or_else(|_| Uuid::new_v4().to_string()));

/// Set by [`crate::TracingConfiguration::setup_tracing_subscriber`] when OpenTelemetry export
/// is enabled. Datapath uses this to skip per-message trace extraction, span creation, and
/// metadata injection when no OTLP pipeline is configured.
static OTEL_PROPAGATION_ENABLED: AtomicBool = AtomicBool::new(false);

/// Returns whether OpenTelemetry propagation (W3C trace context in message metadata) is active.
#[inline]
pub fn otel_propagation_enabled() -> bool {
    OTEL_PROPAGATION_ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn set_otel_propagation_enabled(enabled: bool) {
    OTEL_PROPAGATION_ENABLED.store(enabled, Ordering::Relaxed);
}
