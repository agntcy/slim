# SLIM Tracing Module

[![Version](https://img.shields.io/badge/version-0.2.1-blue.svg)](https://github.com/agntcy/slim/tree/main/data-plane/core/tracing)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-tracing` module provides comprehensive observability for the
SLIM data plane through structured logging, distributed tracing, and metrics. It
offers a flexible configuration system for controlling logging levels and
enabling OpenTelemetry integration.

## Overview

This module serves as the observability foundation for all SLIM components,
enabling:

- Structured logging for debugging and operational insights
- Distributed tracing to track requests across service boundaries
- Metrics collection for performance monitoring and alerts
- Integration with standard observability platforms

The tracing module uses [tracing](https://github.com/tokio-rs/tracing) and
[OpenTelemetry](https://opentelemetry.io/) for a unified approach to
observability, allowing developers to diagnose issues across the SLIM ecosystem.

## Module Structure

### Core Components

- **TracingConfiguration**: Central configuration for all observability settings
- **OpenTelemetryConfig**: Configuration specific to OpenTelemetry setup
- **Subscriber Setup**: Functions for initializing the tracing subscribers
- **Instance ID Management**: Generation and management of unique instance
  identifiers

### Key Files

- **`lib.rs`**: Main implementation of logging and tracing configuration
- **`utils.rs`**: Utilities like instance ID management and context handling

## Features

### Structured Logging

Configure logging levels and formatting:

```rust
let config = TracingConfiguration::default()
    .with_log_level("debug".to_string())
    .with_display_thread_names(true)
    .with_display_thread_ids(true)
    .with_filter("info,slim=debug".to_string());

let _guard = config.setup_tracing_subscriber()?;
```

### Distributed Tracing

Enable OpenTelemetry integration with customized settings:

```rust
let tracing_config = TracingConfiguration::default()
    .with_opentelemetry_config(
        OpenTelemetryConfig::default()
            .with_enabled(true)
            .with_service_name("slim-gateway".to_string())
            .with_service_version("v0.1.0".to_string())
            .with_environment("production".to_string())
            .with_grpc_config(grpc_config)
    );

let _guard = tracing_config.setup_tracing_subscriber()?;
```

### Metrics Collection

The module supports metrics collection with configurable export intervals:

```rust
let config = TracingConfiguration::default()
    .enable_opentelemetry()
    .with_metrics_interval(15); // 15 seconds between metric exports

let _guard = config.setup_tracing_subscriber()?;
```

### Instance Identification

A unique instance ID is generated for each SLIM instance, used to correlate logs
and traces:

```rust
use slim_tracing::utils::INSTANCE_ID;

tracing::info!(instance_id = %INSTANCE_ID.as_str(), "Service started");
```

## Integration with SLIM

The tracing module integrates with all other SLIM components:

- Used by the `service` module to trace request flow
- Integrated with the `datapath` module for network-level traces
- Provides context propagation for distributed tracing across service boundaries
- Used by the `controller` module for management operation tracing

## Configuration

The tracing module can be configured via structured settings:

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct TracingConfiguration {
    // Log level: trace, debug, info, warn, error
    log_level: String,

    // Whether to display thread names in logs
    display_thread_names: bool,

    // Whether to display thread IDs in logs
    display_thread_ids: bool,

    // Filtering expression for logs
    filter: String,

    // OpenTelemetry configuration
    opentelemetry: OpenTelemetryConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OpenTelemetryConfig {
    // Whether OpenTelemetry is enabled
    enabled: bool,

    // gRPC configuration for OpenTelemetry collector
    grpc: ClientConfig,

    // Service name for traces and metrics
    service_name: String,

    // Service version for traces and metrics
    service_version: String,

    // Deployment environment (dev, staging, production)
    environment: String,

    // Interval between metric exports in seconds
    metrics_interval_secs: u64,
}
```

## Usage

To use this module in your Rust project, add it to your `Cargo.toml`:

```toml
[dependencies]
agntcy-slim-tracing = "0.2.1"
```

### Basic Example

```rust
use slim_tracing::TracingConfiguration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the tracing subscriber
    let config = TracingConfiguration::default()
        .with_log_level("debug".to_string())
        .enable_opentelemetry();

    let _guard = config.setup_tracing_subscriber()?;

    // Log messages with structured data
    tracing::info!(target: "slim::app", user_id = 123, "User logged in");

    // Create spans for tracing operations
    let span = tracing::info_span!("process_request", request_id = 456);
    let _guard = span.enter();

    // Child operations inherit the parent context
    tracing::debug!("Processing request details");

    Ok(())
}
```

## OpenTelemetry Integration

### Local Development Setup

To start a complete telemetry stack locally during development:

```bash
task data-plane:telemetry:start
```

This command starts:

1. **OpenTelemetry Collector**: Receives and processes telemetry data
2. **Jaeger**: UI for visualizing and analyzing distributed traces
   (http://localhost:16686)
3. **Prometheus**: Time-series database for metrics (http://localhost:9090)

### Production Deployment

For production environments, configure the module to send telemetry to your
observability platform:

```rust
let config = TracingConfiguration::default()
    .with_opentelemetry_config(
        OpenTelemetryConfig::default()
            .with_enabled(true)
            .with_grpc_config(ClientConfig {
                endpoint: "https://otel-collector.example.com:4317".to_string(),
                tls: TlsClientConfig::new()
                    .with_ca_certificate("path/to/ca.pem".to_string())
                    .with_domain_name("otel-collector.example.com".to_string()),
            })
    );
```

## Advanced Topics

### Context Propagation

The module supports OpenTelemetry context propagation for distributed tracing:

```rust
// Extract context from incoming metadata
let parent_context = extract_context_from_headers(headers);

// Create a span with the parent context
let span = tracing::info_span!("handle_request")
    .with_parent(parent_context);

// Inject context into outgoing metadata
let mut metadata = HashMap::new();
inject_context_into_headers(&mut metadata);
```

### Performance Considerations

- Log filtering is applied at compile time for optimal performance
- OpenTelemetry exports traces asynchronously to avoid blocking application code
- Sampling strategies can be configured for high-traffic environments

## License

This project is licensed under the Apache License, Version 2.0 - see the LICENSE
file for details.

### Using Tracing

To add span instrumentation to your functions:

```rust
#[tracing::instrument]
fn process_request(req_id: &str, payload: &Payload) {
    // Function logic here will be automatically traced
    // with req_id and payload as span attributes
}
```

For more details on instrumentation, see: tracing instrument documentation:
https://docs.rs/tracing/latest/tracing/attr.instrument.html

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

For more details on metrics usage, see: tracing-opentelemetry MetricsLayer
documentation:
https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/struct.MetricsLayer.html#usage
