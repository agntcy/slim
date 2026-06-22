// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser tracing subscriber that pipes `tracing` events to the browser
//! console via `console.log` / `console.warn` / `console.error`.
//!
//! Uses [`tracing_web`] which maps tracing levels to the corresponding
//! `console.*` methods and formats spans using the Performance API.

use thiserror::Error;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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

    tracing_subscriber::registry()
        .with(EnvFilter::new(level))
        .with(fmt_layer)
        .with(perf_layer)
        .try_init()?;

    Ok(())
}
