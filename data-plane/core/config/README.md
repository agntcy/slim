# SLIM Configuration Module

[![Version](https://img.shields.io/badge/version-0.1.8-blue.svg)](https://github.com/agntcy/slim/tree/main/data-plane/core/config)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-config` module provides comprehensive configuration utilities
for the SLIM (Scalable Language Interface Manager) data plane. It offers a
flexible and extensible configuration system that supports various
authentication mechanisms, gRPC communication settings, and TLS security
options.

## Overview

This module is a core component of the SLIM architecture, providing the
configuration infrastructure used throughout the system. It enables:

- Loading configurations from files or environment variables
- Setting up authentication for server and client components
- Configuring gRPC servers and clients with various middleware options
- Managing TLS certificates and security settings
- Defining and validating component configurations

## Module Structure

### Authentication

Authentication mechanisms for SLIM communications:

- **`auth.rs`** - Core authentication interfaces and utilities
- **`auth/basic.rs`** - Username/password authentication implementation
- **`auth/bearer.rs`** - Bearer token-based authentication

### Component Management

Component system used throughout SLIM:

- **`component.rs`** - Base component traits and interfaces
- **`component/configuration.rs`** - Configuration structures and validation
- **`component/id.rs`** - Component identification and namespacing

### gRPC Communication

Configuration for gRPC channels:

- **`grpc.rs`** - Core gRPC configuration interfaces
- **`grpc/client.rs`** - Client-side gRPC configuration
- **`grpc/compression.rs`** - Message compression settings
- **`grpc/errors.rs`** - Error handling for gRPC operations
- **`grpc/headers_middleware.rs`** - Middleware for gRPC header manipulation
- **`grpc/server.rs`** - Server-side gRPC configuration

### Configuration Providers

Sources for configuration data:

- **`provider.rs`** - Provider interfaces and resolver utilities
- **`provider/env.rs`** - Environment variable-based configuration provider
- **`provider/file.rs`** - YAML file-based configuration provider

### TLS Security

TLS certificate and security settings:

- **`tls.rs`** - Core TLS configuration interfaces
- **`tls/client.rs`** - Client-side TLS configuration and certificate handling
- **`tls/common.rs`** - Shared TLS functionality and utilities
- **`tls/server.rs`** - Server-side TLS certificate management

### Utilities

- **`testutils.rs`** - Utilities for testing configurations
- **`build.rs`** - Build script for the configuration module

## Integration with SLIM

The configuration module is used extensively throughout the SLIM system:

- The SLIM binary uses it to parse command-line configuration files
- The service module uses it for server and client connection setup
- Component configurations are validated using its utilities
- Python bindings leverage it for configuration management

## Dependency

To use this module in your Rust project, add it to your `Cargo.toml`:

```toml
[dependencies]
agntcy-slim-config = "0.1.8"
```
