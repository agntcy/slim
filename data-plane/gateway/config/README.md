# Configuration Module

This module provides configuration utilities for the gateway. It includes
various components for authentication, gRPC, and TLS settings.

## Files

### `auth.rs`

This file contains the main authentication logic and structures.

### `auth/basic.rs`

This file provides basic authentication mechanisms.

### `auth/bearer.rs`

This file implements bearer token authentication.

### `component.rs`

This file defines the main components used in the configuration.

### `component/configuration.rs`

This file contains the configuration structures and logic for components.

### `component/id.rs`

This file provides utilities for handling component IDs.

### `grpc.rs`

This file contains the main gRPC configuration logic.

### `grpc/client.rs`

This file provides client-side gRPC configuration.

### `grpc/compression.rs`

This file implements gRPC compression settings.

### `grpc/errors.rs`

This file defines error handling for gRPC.

### `grpc/headers_middleware.rs`

This file provides middleware for handling gRPC headers.

### `grpc/server.rs`

This file contains server-side gRPC configuration.

### `provider.rs`

This file defines the main configuration providers.

### `provider/env.rs`

This file provides environment variable-based configuration.

### `provider/file.rs`

This file provides file-based configuration.

### `testutils.rs`

This file contains utilities for testing configurations.

### `tls.rs`

This file contains the main TLS configuration logic.

### `tls/client.rs`

This file provides client-side TLS configuration.

### `tls/common.rs`

This file contains common TLS utilities.

### `tls/server.rs`

This file provides server-side TLS configuration.

### `build.rs`

This file contains the build script for the configuration module.

## Usage

To use this module, include it in your `Cargo.toml`:

```toml
[dependencies]
gateway-config = "0.1.0"
```
