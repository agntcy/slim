// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser tracing subscriber that pipes `tracing` events to the browser
//! console via `console.log` / `console.warn` / `console.error`.
//!
//! Uses [`tracing_web`] which maps tracing levels to the corresponding
//! `console.*` methods and formats spans using the Performance API.
//!
//! With the `otel_tracing` feature, a lightweight in-process tracer provider
//! installs the W3C [`TraceContextPropagator`] and bridges `tracing` spans to
//! OpenTelemetry context. Spans are not exported to a collector (browser OTLP
//! export is out of scope); propagation into SLIM message metadata is handled
//! by `slim_datapath` when built with its `otel_tracing` feature.

use thiserror::Error;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "otel_tracing")]
use opentelemetry::global;
#[cfg(feature = "otel_tracing")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel_tracing")]
use opentelemetry_sdk::propagation::TraceContextPropagator;
#[cfg(feature = "otel_tracing")]
use opentelemetry_sdk::trace::TracerProviderBuilder;
#[cfg(feature = "otel_tracing")]
use tracing_opentelemetry::OpenTelemetryLayer;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("error setting up tracing subscriber")]
    TracingSetupError(#[from] tracing_subscriber::util::TryInitError),

    #[error("error parsing filter directives")]
    FilterParseError(#[from] tracing_subscriber::filter::ParseError),
}

/// Initialize browser console logging with the given log level filter.
///
/// Call once at application startup (e.g. from `wasm_bindgen(start)`).
///
/// When compiled with the `otel_tracing` feature, also installs the global W3C
/// trace-context propagator and an in-process OpenTelemetry tracer so
/// `slim_datapath` can inject and extract parent spans on SLIM messages. Enable
/// `otel_tracing` on both `slim_tracing` and `slim_datapath` in browser apps
/// that participate in distributed traces.
///
/// # Arguments
/// * `level` — a `tracing` filter directive string (e.g. `"info"`,
///   `"slim_session=debug,info"`). Falls back to `"info"` if empty.
///
/// # Example
/// ```ignore
/// slim_tracing::init_tracing("debug").expect("tracing init failed");
/// ```
pub fn init_tracing(level: &str) -> Result<(), ConfigError> {
    let level = if level.is_empty() { "info" } else { level };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_writer(tracing_web::MakeConsoleWriter);

    let perf_layer = tracing_web::performance_layer()
        .with_details_from_fields(tracing_subscriber::fmt::format::Pretty::default());

    #[cfg(feature = "otel_tracing")]
    {
        global::set_text_map_propagator(TraceContextPropagator::new());

        // No exporter: spans exist for in-browser visibility and for metadata
        // propagation into SLIM messages. OTLP export from wasm is not supported.
        let provider = TracerProviderBuilder::default().build();
        let tracer = provider.tracer("slim-browser");

        tracing_subscriber::registry()
            .with(EnvFilter::new(level))
            .with(fmt_layer)
            .with(perf_layer)
            .with(OpenTelemetryLayer::new(tracer))
            .try_init()?;
    }

    #[cfg(not(feature = "otel_tracing"))]
    {
        tracing_subscriber::registry()
            .with(EnvFilter::new(level))
            .with(fmt_layer)
            .with(perf_layer)
            .try_init()?;
    }

    Ok(())
}
