# Datapath Module

This module provides the core functionalities for the gateway's data path.
It includes various components for handling messages, connections,
and pub/sub mechanisms.

## Files

### `connection.rs`

This file contains the logic for managing connections.

### `errors.rs`

This file defines error handling for the data path.

### `forwarder.rs`

This file implements the forwarding logic for messages.

### `lib.rs`

This file contains the main entry point for the data path module.

### `message_processing.rs`

This file provides utilities for processing messages.

### `messages.rs`

This file defines the structures and logic for handling messages.

### `messages/encoder.rs`

This file provides encoding utilities for messages.

### `messages/utils.rs`

This file contains utility functions for message handling.

### `pubsub.rs`

This file implements the publish-subscribe mechanism.

### `pubsub/proto.rs`

This file defines the protocol buffer for the pub/sub mechanism.

### `tables.rs`

This file contains the logic for managing various tables used in the data path.

### `tables/connection_table.rs`

This file defines the connection table management logic.

### `tables/errors.rs`

This file provides error handling for table operations.

### `tables/subscription_table.rs`

This file implements the subscription table logic.

### `tables/pool.rs`

This file contains the logic for managing object pools.

### `pubsub/gen/pubsub.proto.v1.rs`

This file contains the generated code for the pub/sub protocol buffer.

## Usage

To use this module, include it in your `Cargo.toml`:

```toml
[dependencies]
gateway-datapath = "0.1.0"
```
