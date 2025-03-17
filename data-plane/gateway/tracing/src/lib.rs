// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_sdk::{
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::attribute::{
    DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION,
};

use serde::{Deserialize, Serialize};
use tracing::Level;
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::{
    filter::LevelFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

pub mod opaque;
pub mod utils;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TracingConfiguration {
    #[serde(default = "default_log_level")]
    log_level: String,

    #[serde(default = "default_display_thread_names")]
    display_thread_names: bool,

    #[serde(default = "default_display_thread_ids")]
    display_thread_ids: bool,

    #[serde(default = "default_filter")]
    filter: String,

    #[serde(default)]
    opentelemetry: OpenTelemetryConfig,
}

// default implementation for TracingConfiguration
impl Default for TracingConfiguration {
    fn default() -> Self {
        TracingConfiguration {
            log_level: default_log_level(),
            display_thread_names: default_display_thread_names(),
            display_thread_ids: default_display_thread_ids(),
            filter: default_filter(),
            opentelemetry: OpenTelemetryConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenTelemetryConfig {
    #[serde(default)]
    enabled: bool,

    #[serde(default = "default_service_name")]
    service_name: String,

    #[serde(default = "default_service_version")]
    service_version: String,

    #[serde(default = "default_environment")]
    environment: String,

    #[serde(default = "default_metrics_interval")]
    metrics_interval_secs: u64,
}

// default implementation for OpenTelemetryConfig
impl Default for OpenTelemetryConfig {
    fn default() -> Self {
        OpenTelemetryConfig {
            enabled: false,
            service_name: default_service_name(),
            service_version: default_service_version(),
            environment: default_environment(),
            metrics_interval_secs: default_metrics_interval(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_display_thread_names() -> bool {
    true
}

fn default_display_thread_ids() -> bool {
    false
}

fn default_filter() -> String {
    "info".to_string()
}

fn default_service_name() -> String {
    "agp-data-plane".to_string()
}

fn default_service_version() -> String {
    "v0.1.0".to_string()
}

fn default_environment() -> String {
    "development".to_string()
}

fn default_metrics_interval() -> u64 {
    30 // default to 30 seconds
}

// function to convert string tracing level to tracing::Level
fn resolve_level(level: &str) -> tracing::Level {
    let level = level.to_lowercase();
    match level.as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO, // default level
    }
}

pub struct OtelGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(tracer) = self.tracer_provider.take() {
            if let Err(err) = tracer.shutdown() {
                eprintln!("Error shutting down tracer provider: {err:?}");
            }
        }
        if let Some(meter) = self.meter_provider.take() {
            if let Err(err) = meter.shutdown() {
                eprintln!("Error shutting down meter provider: {err:?}");
            }
        }
    }
}

impl TracingConfiguration {
    pub fn with_log_level(self, log_level: String) -> Self {
        TracingConfiguration { log_level, ..self }
    }

    pub fn with_display_thread_names(self, display_thread_names: bool) -> Self {
        TracingConfiguration {
            display_thread_names,
            ..self
        }
    }

    pub fn with_display_thread_ids(self, display_thread_ids: bool) -> Self {
        TracingConfiguration {
            display_thread_ids,
            ..self
        }
    }

    pub fn with_filter(self, filter: String) -> Self {
        TracingConfiguration { filter, ..self }
    }

    pub fn with_opentelemetry_config(mut self, config: OpenTelemetryConfig) -> Self {
        self.opentelemetry = config;
        self
    }

    pub fn enable_opentelemetry(mut self) -> Self {
        self.opentelemetry.enabled = true;
        self
    }

    pub fn with_metrics_interval(mut self, interval_secs: u64) -> Self {
        self.opentelemetry.metrics_interval_secs = interval_secs;
        self
    }

    pub fn log_level(&self) -> &str {
        &self.log_level
    }

    pub fn display_thread_names(&self) -> bool {
        self.display_thread_names
    }

    pub fn display_thread_ids(&self) -> bool {
        self.display_thread_ids
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Set up a subscriber
    pub fn setup_tracing_subscriber(&self) -> OtelGuard {
        let fmt_layer = fmt::layer()
            .with_thread_ids(self.display_thread_ids)
            .with_thread_names(self.display_thread_names)
            .with_filter(tracing_subscriber::filter::filter_fn(
                |metadata: &tracing::Metadata| {
                    !metadata
                        .fields()
                        .iter()
                        .any(|field| field.name() == "telemetry")
                },
            ));

        let level_filter = LevelFilter::from_level(resolve_level(&self.log_level));

        if self.opentelemetry.enabled {
            // resource
            let resource = Resource::builder()
                .with_attributes([
                    KeyValue::new(SERVICE_NAME, self.opentelemetry.service_name.clone()),
                    KeyValue::new(SERVICE_VERSION, self.opentelemetry.service_version.clone()),
                    KeyValue::new(
                        DEPLOYMENT_ENVIRONMENT_NAME,
                        self.opentelemetry.environment.clone(),
                    ),
                ])
                .build();

            // init tracer provider
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .build()
                .unwrap();

            let tracer_provider = SdkTracerProvider::builder()
                // TODO(zkacsand): customize sampling strategy
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
                    1.0,
                ))))
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource.clone())
                .with_batch_exporter(exporter)
                .build();

            // init meter provider
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_temporality(opentelemetry_sdk::metrics::Temporality::default())
                .build()
                .unwrap();

            let reader = PeriodicReader::builder(exporter)
                .with_interval(std::time::Duration::from_secs(
                    self.opentelemetry.metrics_interval_secs,
                ))
                .build();

            let stdout_reader =
                PeriodicReader::builder(opentelemetry_stdout::MetricExporter::default()).build();

            let meter_provider = MeterProviderBuilder::default()
                .with_resource(resource.clone())
                .with_reader(reader)
                .with_reader(stdout_reader)
                .build();

            // set global meter provider
            global::set_meter_provider(meter_provider.clone());

            // Sst up the trace context propagator
            let propagator = opentelemetry_sdk::propagation::TraceContextPropagator::new();
            global::set_text_map_propagator(propagator);

            let tracer = tracer_provider.tracer("tracing-otel-subscriber");

            // Construct the subscriber with OpenTelemetry
            tracing_subscriber::registry()
                .with(level_filter)
                .with(fmt_layer)
                .with(MetricsLayer::new(meter_provider.clone()))
                .with(OpenTelemetryLayer::new(tracer))
                .init();

            OtelGuard {
                tracer_provider: Some(tracer_provider),
                meter_provider: Some(meter_provider),
            }
        } else {
            // Basic subscriber without OpenTelemetry
            tracing_subscriber::registry()
                .with(level_filter)
                .with(fmt_layer)
                .init();

            OtelGuard {
                tracer_provider: None,
                meter_provider: None,
            }
        }
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tracing_configuration() {
        let config = TracingConfiguration::default();
        assert_eq!(config.log_level, default_log_level());
        assert_eq!(config.display_thread_names, default_display_thread_names());
        assert_eq!(config.display_thread_ids, default_display_thread_ids());
        assert_eq!(config.filter, default_filter());
    }

    #[test]
    fn test_resolve_level() {
        assert_eq!(resolve_level("trace"), Level::TRACE);
        assert_eq!(resolve_level("debug"), Level::DEBUG);
        assert_eq!(resolve_level("info"), Level::INFO);
        assert_eq!(resolve_level("warn"), Level::WARN);
        assert_eq!(resolve_level("error"), Level::ERROR);
        assert_eq!(resolve_level("invalid"), Level::INFO);
    }
}
