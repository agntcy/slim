# SLIM-Signal Module

[![Version](https://img.shields.io/badge/version-0.1.2-blue.svg)](https://github.com/agntcy/slim)
[![License](https://img.shields.io/badge/license-Apache_2.0-green.svg)](https://github.com/agntcy/slim/blob/main/LICENSE)

A small, cross-platform library for handling OS signals in SLIM applications.
This module provides a unified API for signal handling across both Unix and
Windows platforms, enabling graceful application shutdown.

## Overview

The SLIM-Signal module abstracts platform-specific signal handling, offering a
simple interface to wait for and react to OS termination signals. Key features
include:

- Cross-platform signal handling with unified API
- Support for Unix-specific signals (SIGINT, SIGTERM)
- Windows Ctrl+C signal support
- Integration with Tokio for asynchronous operation
- Tracing integration for debugging and monitoring

## Module Structure

### Core Components

- **Public API**: Simple interface to wait for and handle shutdown signals
- **Platform-specific implementations**:
  - Unix implementation (handles SIGINT and SIGTERM)
  - Windows implementation (handles Ctrl+C events)

### Implementation Details

The module is organized into:

- **Main API**: The `shutdown()` function that blocks until a termination signal
  is received
- **Platform-specific modules**: Separate implementation modules for Unix and
  Windows systems using conditional compilation

## Usage

To use this module, include it in your `Cargo.toml`:

```toml
[dependencies]
agntcy-slim-signal = "0.1.2"
```

### Basic Example

```rust
use slim_signal;

#[tokio::main]
async fn main() {
    // Initialize your application...

    // Wait for a shutdown signal
    tokio::select! {
        _ = slim_signal::shutdown() => {
            println!("Shutdown signal received, starting cleanup...");
        }
        // Other application tasks...
    }

    // Perform cleanup operations...
}
```

### Integration with SLIM

The signal module is typically used in SLIM applications to handle graceful
shutdown:

```rust
tokio::select! {
    _ = slim_signal::shutdown() => {
        info!("Received shutdown signal");

        // Send drain signals to services
        for service in services {
            let signal = service.signal();
            match timeout(config.drain_timeout(), signal.drain()).await {
                Ok(()) => {},
                Err(_) => error!("Timeout waiting for service to drain"),
            }
        }
    }
}
```

## Dependencies

- **tokio**: For async signal handling and event management
- **tracing**: For logging signal events

## Platform Support

- **Unix/Linux/macOS**: Handles SIGINT (Ctrl+C) and SIGTERM signals
- **Windows**: Handles Ctrl+C events

## License

This module is licensed under the Apache License, Version 2.0.
