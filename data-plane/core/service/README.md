# SLIM Service Module

[![Version](https://img.shields.io/badge/version-0.4.2-blue.svg)](https://github.com/agntcy/slim/tree/main/data-plane/core/service)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-service` module provides the high-level API and session
management layer for SLIM applications. It serves as the main entry point for
integrating with the SLIM data plane, offering abstractions for various
communication patterns and session management.

## Overview

This module bridges application logic with the underlying SLIM data plane,
providing:

- High-level interface for applications to interact with SLIM
- Session management for different communication patterns
- Connection establishment and management
- Message sending and receiving logic
- Routing configuration for messaging patterns

The service layer translates application-level operations into the appropriate
network-level operations handled by the datapath module.

## Module Structure

### Core Components

- **Service**: Main entry point that manages connections, sessions, and message
  routing
- **Session Layer**: Manages session pools and handles session lifecycle events
- **Sessions**: Different session implementations for various communication
  patterns

### Communication Patterns

The service module supports several communication patterns through session
types:

1. **Request/Response**: Synchronous call-and-response pattern with timeout
   handling

   ```rust
   // Creating a request/response session
   let session_info = service
       .create_session(&agent, SessionConfig::RequestResponse(RequestResponseConfiguration {
           timeout: std::time::Duration::from_millis(1000),
       }))
       .await?;
   ```

2. **Fire and Forget**: Simple message delivery with optional reliability

   ```rust
   // Creating a fire-and-forget session
   let session_info = service
       .create_session(&agent, SessionConfig::FireAndForget(FireAndForgetConfiguration {
           timeout: std::time::Duration::from_millis(1000),
           max_retries: Some(3),
           sticky: false,
       }))
       .await?;
   ```

3. **Streaming**: Continuous data stream with reliability mechanisms

   ```rust
   // Creating a streaming session
   let session_info = service
       .create_session(&agent, SessionConfig::Streaming(StreamingConfiguration::new(
           SessionDirection::Sender,
           None,
           Some(10),
           Some(std::time::Duration::from_millis(1000)),
       )))
       .await?;
   ```

4. **Pub/Sub**: Many-to-many event publishing and subscription

### Key Components

- **Session Management**: Create, configure, and manage communication sessions
- **Buffer Management**: Handle message buffering for reliable delivery
- **Timers and Retransmission**: Manage message timeouts and retransmissions
- **Connection Handling**: Establish and maintain connections to other SLIM
  instances

## Key Features

### Session Configuration

The service layer allows for detailed configuration of sessions:

```rust
// Set session configuration
service.set_session_config(
    &agent,
    &session_config,
    Some(session_id),
).await?;

// Get session configuration
let config = service.get_session_config(
    &agent,
    session_id,
).await?;
```

### Dynamic Routing

Configure dynamic message routing between agents:

```rust
// Subscribe to messages
service.subscribe(
    &local_agent,
    &agent_type,
    Some(agent_id),
    Some(conn_id),
).await?;

// Set a specific route
service.set_route(
    &local_agent,
    &agent_type,
    Some(agent_id),
    conn_id,
).await?;
```

### Message Publishing

Send messages through various session types:

```rust
// Publish a message
service.publish(
    &source,
    session_info,
    &destination_agent_type,
    Some(destination_agent_id),
    data,
).await?;

// Publish to a specific connection
service.publish_to(
    &source,
    session_info,
    &destination_agent_type,
    Some(destination_agent_id),
    connection_id,
    data,
).await?;
```

## Integration with SLIM

The service module integrates with other SLIM components:

- Uses the `datapath` module for messaging and routing
- Relies on the `config` module for secure connection settings
- Integrates with the `controller` for management capabilities
- Provides a high-level interface consumed by client applications and bindings

## Usage

To use this module in your Rust project, add it to your `Cargo.toml`:

```toml
[dependencies]
agntcy-slim-service = "0.4.2"
```

### Basic Example

```rust
use slim_service::{Service, session};
use slim_datapath::messages::Agent;

// Create a service configuration
let config = ServiceConfiguration::new()
    .with_server(vec![server_config])
    .with_client(vec![client_config]);

// Create the service
let service = Service::new()
    .await?;

// Register a local agent
let agent = Agent::from_strings("organization", "namespace", "agent", 0);
service.register(&agent).await?;

// Connect to a remote server
let conn_id = service.connect(&client_config).await?;

// Create a session for communication
let session_info = service
    .create_session(&agent, session::SessionConfig::RequestResponse(
        RequestResponseConfiguration::default()
    ))
    .await?;

// Publish a message
service.publish(
    &agent,
    session_info,
    &destination_agent_type,
    Some(destination_agent_id),
    data,
).await?;

// Receive a message
let (session_info, message) = rx.recv().await?;
```

## Error Handling

The service module provides detailed error types through the `ServiceError`
enum:

- `ConnectionError`: Issues when establishing connections
- `SessionError`: Problems with session creation or management
- `MessageSendingError`: Failures when sending messages
- `AgentNotFound`: Agent wasn't registered with the service
- `Timeout`: Request/response operations timeout

## Advanced Capabilities

- **Controller Integration**: Connect to the SLIM control plane
- **Multiple Servers**: Run multiple server endpoints
- **Custom Serialization**: Use any format for message payloads
- **Session Deletion**: Clean up resources when sessions are no longer needed

## License

This project is licensed under the Apache License, Version 2.0 - see the LICENSE
file for details.
