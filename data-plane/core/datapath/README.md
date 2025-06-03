# SLIM Datapath Module

[![Version](https://img.shields.io/badge/version-0.7.0-blue.svg)](https://github.com/agntcy/slim/tree/main/data-plane/core/datapath)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-datapath` module serves as the foundation of the SLIM
communication system, providing the core networking infrastructure for message
routing, connection management, and multiple communication patterns.

## Overview

The datapath module manages the flow of messages between SLIM instances,
handling the low-level transport and routing mechanisms. It implements several
key features:

- Message routing based on organization, namespace, and agent identifiers
- Connection management for reliable communication
- Flexible subscription model for dynamic service discovery
- Protocol buffer-based message serialization
- Multiple session patterns (request/reply, streaming, fire-and-forget, pubsub)
- Support for message retransmission and reliability mechanisms
- Connection pooling and resource management

## Module Structure

### Core Components

- **Message Processor**: Central component that handles message flow between
  instances
- **Forwarder**: Routes messages to appropriate connections
- **Connection Table**: Manages active connections
- **Subscription Table**: Tracks subscriptions for routing

### Communication Patterns

The datapath module supports multiple communication patterns implemented through
session types:

- **Request/Reply**: Synchronous request/response pattern with timeout
  management
- **Streaming**: Ordered message delivery with retransmission for reliable
  streaming
- **Fire and Forget**: Simple message delivery with optional reliability
- **Pub/Sub**: Many-to-many communication pattern for event distribution

### Protocol Implementation

- **Protocol Buffers**: Defines the wire format for all messages
- **Session Headers**: Manages session types, IDs, and message metadata
- **SLIM Headers**: Handles routing information and connection details

## Key Features

### Flexible Routing and Subscription

The datapath supports dynamic routing based on agent identifiers:

- Dynamic subscription table for efficient message routing
- Connection pooling for optimized resource utilization
- Reference counting for connection management
- Organization and namespace-based routing
- Hierarchical addressing for flexible message delivery

### OpenTelemetry Integration

The module includes built-in OpenTelemetry tracing support:

- Automatic propagation of tracing context across service boundaries
- Metadata extraction from incoming messages
- Span creation and management for message processing
- Distributed tracing across multiple SLIM instances
- Integration with OpenTelemetry exporters

### Message Processing Pipeline

The module implements a sophisticated message processing pipeline:

- Type-based message dispatching
- Support for multiple message patterns (publish/subscribe, request/response)
- Error handling and status reporting
- Asynchronous message processing
- Graceful handling of routing edge cases
- Detailed logging and diagnostics

### Connection Management

The module provides robust connection management:

- Connection type classification (remote, local)
- Address tracking for both local and remote endpoints
- Channel management for bidirectional communication
- Cancellation token support for graceful shutdown
- Connection establishment and teardown handling
- Connection indexing for quick lookups

## Integration with SLIM

The datapath module integrates with other SLIM components:

- Used by the `service` module to provide higher-level session abstractions
- Configured through the `config` module for secure communication settings
- Connected to the control plane through the `controller` module
- Provides tracing integration with the `tracing` module

## Performance Considerations

The module is designed for high performance with features like:

- Connection pooling for efficient resource usage
- Non-blocking asynchronous I/O with Tokio
- Optimized message serialization with Protocol Buffers
- Efficient routing tables for quick message delivery

## License

This module is licensed under the Apache License, Version 2.0.
