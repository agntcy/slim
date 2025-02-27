# Tracing Module

This module provides tracing and observability functionalities for the gateway. It includes the
main entry point for the tracing logic and additional utilities.

## Files

### `lib.rs`
This file contains the main entry point for the tracing module.

### `opaque.rs`
This file provides utilities for handling opaque tracing data.

## Usage

To use this module, include it in your `Cargo.toml`:

```toml
[dependencies]
gateway-tracing = "0.1.0"
```

## Configuration

The module provides a TracingConfiguration struct for configuring logging and tracing:

```rust
let config = TracingConfiguration::default()
    .with_log_level("debug".to_string())
    .enable_opentelemetry();

let _guard = config.setup_tracing_subscriber();
```

The tracing subscriber must be set up inside a tokio runtime.

## OpenTelemetry integration

The module includes built-in support for OpenTelemetry, enabling distributed tracing and metrics collection.

### Local development

To start the telemetry stack locally during development (otel-collector, Jaeger, Prometheus):

`task data-plane:telemetry:start`

This will set up:
- OpenTelemetry Collector for receiving and processing telemetry data
- Jaeger for trace visualization and analysis
- Prometheus for metrics collection and monitoring

### Using Tracing

To add span instrumentation to your functions:

```rust
#[tracing::instrument]
fn process_request(req_id: &str, payload: &Payload) {
    // Function logic here will be automatically traced
    // with req_id and payload as span attributes
}
```

For more details on instrumentation, see: tracing instrument documentation: https://docs.rs/tracing/latest/tracing/attr.instrument.html

You can also create manual spans:

```rust
use tracing::{info, info_span};

let span = info_span!("processing", request_id = req_id);
let _guard = span.enter();

// Operations inside this scope will be captured in the span
info!("Starting processing");
```

### Using Metrics

Metrics can be recorded directly using the tracing macros:

```rust
use tracing::info;

// Record a counter metric
info!(counter.num_active_connections = 1);
```

For more details on metrics usage, see: tracing-opentelemetry MetricsLayer documentation: https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/struct.MetricsLayer.html#usage
