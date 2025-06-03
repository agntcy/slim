# SLIM Module

![Version](https://img.shields.io/badge/version-0.3.15-blue)
![License](https://img.shields.io/badge/license-Apache--2.0-green)

The SLIM (Scalable Low-latency Interactive Messaging) Module is the core
executable package of the SLIM framework. It provides the primary runtime
environment, configuration handling, and service orchestration capabilities that
power the entire SLIM ecosystem.

## Overview

SLIM is designed as a modular, high-performance messaging framework that
enables:

- Fast, low-latency communication between distributed components
- Configurable service architecture through a unified configuration system
- Runtime management of multiple services with efficient resource utilization
- Comprehensive observability and telemetry

This module serves as the main entry point for SLIM applications, orchestrating
the entire lifecycle from startup to graceful shutdown.

## Core Components

### Runtime Management

The runtime component (`runtime.rs`) provides:

- Configurable multi-core execution environment using Tokio
- Graceful shutdown with configurable drain timeout
- Efficient thread management and resource allocation

```rust
// Example runtime configuration
let runtime_config = RuntimeConfiguration {
    n_cores: 4,                            // Number of cores to use
    thread_name: "slim-worker".to_string(), // Thread name prefix
    drain_timeout: Duration::from_secs(10), // Service drain timeout
};
```

### Configuration System

The configuration component (`config.rs`) offers:

- YAML-based configuration parsing and validation
- Dynamic resolution of configuration values
- Component and service configuration management
- Error handling for invalid configurations

```rust
// Loading configuration from a file
let config_result = load_config("path/to/config.yaml")?;
```

### Command-line Interface

The arguments component (`args.rs`) provides:

- Command-line argument parsing using clap
- Support for configuration file specification
- Version information display

```
$ slim --config path/to/config.yaml
$ slim --version
```

### Build Information

The build info component (`build_info.rs`) provides:

- Version tracking and display
- Build date and environment information
- Git commit identification

## Usage

### As a Dependency

To use SLIM as a dependency in your Rust project:

```toml
[dependencies]
agntcy-slim = "0.3.15"
```

### As an Executable

The SLIM package includes a binary that can be run directly:

```sh
# Run with configuration file
slim --config config.yaml

# Display version information
slim --version
```

## Integration with Other SLIM Components

SLIM integrates with the following core components:

- `slim-config`: For component configuration management
- `slim-service`: For service definition and lifecycle management
- `slim-signal`: For signal handling and process management
- `slim-tracing`: For observability and telemetry

## Advanced Configuration

SLIM supports advanced configuration options including:

- Custom runtime settings for thread and resource management
- Comprehensive tracing and observability configuration
- Dynamic service definition and orchestration

See the reference configuration files in the `data-plane/config/reference/`
directory for examples.

## Platform-Specific Optimizations

SLIM includes platform-specific optimizations:

- Memory allocator optimizations on Linux with jemalloc
- Multi-core scalability with configurable thread count
- Efficient resource management based on available system resources

## Resources

- [GitHub Repository](https://github.com/agntcy/slim)
- [Documentation](https://docs.agntcy.ai/slim/)
- [Examples](https://github.com/agntcy/slim/tree/main/data-plane/examples)
