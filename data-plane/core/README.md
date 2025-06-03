# SLIM Core Modules

![Version](https://img.shields.io/badge/version-0.3.x-blue)
![License](https://img.shields.io/badge/license-Apache--2.0-green)

This directory contains the core modules that make up the foundation of the SLIM
(Scalable Low-latency Interactive Messaging) framework. These modules work
together to provide a robust, high-performance messaging system designed for
scalability and low latency communication.

## Overview

SLIM is an advanced messaging framework designed with the following principles:

- **Modularity**: Components can be composed to build custom messaging solutions
- **Performance**: Optimized for low-latency, high-throughput communications
- **Reliability**: Built-in error handling and recovery mechanisms
- **Extensibility**: Easily extensible architecture for custom implementations
- **Observability**: Comprehensive tracing and monitoring capabilities

## Core Modules

The SLIM core is composed of the following modules:

### [slim](./slim/)

The main executable package that serves as the entry point for the SLIM
framework. It provides runtime environment, configuration handling, and service
orchestration.

### [slim-config](./config/)

Configuration management system supporting YAML configuration files, environment
variables, and runtime configuration.

### [slim-controller](./controller/)

Control plane interaction module for managing SLIM instances and components from
a centralized control plane.

### [slim-datapath](./datapath/)

High-performance data path implementation for efficient message routing and
processing.

### [slim-service](./service/)

Service definition and lifecycle management framework for creating messaging
services.

### [slim-signal](./signal/)

Signal handling utilities for graceful startup and shutdown of services.

### [slim-tracing](./tracing/)

Observability module providing logging, metrics, and distributed tracing
capabilities.

### [nop_component](./nop_component/)

Reference implementation of a minimal SLIM component useful for testing and as a
template for custom components.

## Architecture

The SLIM framework follows a layered architecture:

1. **Core Layer**: Provides fundamental functionality (signal handling,
   configuration)
2. **Service Layer**: Manages service definitions and lifecycle
3. **Data Path Layer**: Handles message routing and processing
4. **Extension Layer**: Provides extensibility through custom components

Each module has its own comprehensive documentation in its respective directory.

## Getting Started

For detailed information and usage examples, please refer to the README of each
individual module:

- [slim](./slim/README.md)
- [slim-config](./config/README.md)
- [slim-controller](./controller/README.md)
- [slim-datapath](./datapath/README.md)
- [slim-service](./service/README.md)
- [slim-signal](./signal/README.md)
- [slim-tracing](./tracing/README.md)
- [nop_component](./nop_component/README.md)

## Using SLIM in Your Project

To use SLIM as a dependency in your Rust project:

```toml
[dependencies]
# Main SLIM package
agntcy-slim = "0.3.15"

# Optional individual modules
agntcy-slim-config = "0.1.8"
agntcy-slim-controller = "0.1.0"
agntcy-slim-datapath = "0.3.15"
agntcy-slim-service = "0.3.15"
agntcy-slim-signal = "0.1.8"
agntcy-slim-tracing = "0.1.8"
```

## Resources

- [GitHub Repository](https://github.com/agntcy/slim)
- [Documentation](https://docs.agntcy.ai/slim/)
- [Examples](https://github.com/agntcy/slim/tree/main/data-plane/examples)
