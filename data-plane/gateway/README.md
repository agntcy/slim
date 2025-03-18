# Gateway Module

This module provides the core functionalities for the gateway. It includes
various components for handling arguments, build information, configuration,
and runtime operations.

## Files

### `args.rs`

This file contains the logic for parsing and handling command-line arguments.

### `build_info.rs`

This file provides build information such as version, profile, and git SHA.

### `config.rs`

This file contains the configuration structures and logic for the gateway.

### `lib.rs`

This file contains the main entry point for the gateway module.

### `runtime.rs`

This file defines the runtime operations and logic for the gateway.

## Usage

To use this module, include it in your `Cargo.toml`:

```toml
[dependencies]
gateway = "0.1.0"
```

