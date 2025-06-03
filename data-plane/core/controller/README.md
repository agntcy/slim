# SLIM Controller Module

[![Version](https://img.shields.io/badge/version-0.1.1-blue.svg)](https://github.com/agntcy/slim/tree/main/data-plane/core/controller)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-controller` module provides the control API and service for
configuring and managing the SLIM data plane through the control plane. It
enables dynamic configuration of routing, connections, and subscriptions via a
bidirectional gRPC streaming interface.

## Overview

This module serves as the management layer for SLIM, allowing control plane
components (like `slimctl`) to configure and monitor SLIM data plane instances
at runtime. Key functionalities include:

- Establishing bidirectional control channels between control and data planes
- Dynamic management of connections and routes
- Subscription handling and configuration
- Real-time monitoring of SLIM components
- Status reporting and acknowledgment mechanisms

## Module Structure

### Core Components

- **`lib.rs`** - Main entry point exposing the module's public API
- **`service.rs`** - Implementation of the controller service and its core logic
- **`errors.rs`** - Error types and handling for controller operations

### API Components

- **`api.rs`** - API interfaces and protocol bindings
- **`api/proto.rs`** - Protocol buffer interface definitions
- **`api/gen/controller.proto.v1.rs`** - Generated gRPC code from protobuf
  definitions

### Protocol Definition

The module defines a gRPC service with a bidirectional streaming interface:

- **`ControllerService`** - Main service interface
- **`OpenControlChannel`** - Bidirectional streaming RPC for sending/receiving
  control messages
- **`ControlMessage`** - Container for various control operations with different
  payload types

## Key Features

### Connection Management

The controller enables dynamic creation and management of connections between
SLIM instances, allowing for flexible network topologies:

```rust
// Example from service.rs
match payload {
    control_message::Payload::ConfigCommand(config) => {
        for conn in &config.connections_to_create {
            let client_endpoint = format!("{}:{}", conn.remote_address, conn.remote_port);
            // Connection establishment logic
        }
    }
}
```

### Subscription Configuration

Users can dynamically configure routing by managing subscriptions for specific
organizations, namespaces, and agent types:

```rust
// Example of handling subscription configuration
for sub in &config.subscriptions_to_set {
    let organization = sub.organization.clone();
    let namespace = sub.namespace.clone();
    let agent_type = sub.agent_type.clone();
    let agent_id = sub.agent_id.as_ref().map(|id| id.value);

    // Add subscription to routing table
}
```

### Real-time Feedback

The controller provides acknowledgment messages for operations, allowing clients
to track the status of their requests:

```rust
// Example of sending an acknowledgment
let ack = Ack {
    original_message_id: msg.message_id.clone(),
    success: true,
    messages: Vec::new(),
};

// Send acknowledgment back to client
```

## Integration with SLIM

The controller module is integrated with other SLIM components:

- Works with the `MessageProcessor` from the `datapath` module to route messages
- Uses configuration from `slim-config` for secure communication
- Implements the controller API defined in protocol buffers
- Provides services that can be registered with gRPC servers

## Dependency

To use this module in your Rust project, add it to your `Cargo.toml`:

```toml
[dependencies]
agntcy-slim-controller = "0.1.1"
```

## License

This project is licensed under the Apache License, Version 2.0 - see the LICENSE
file for details.
