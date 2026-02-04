# SLIM Node.js Bindings

Node.js bindings for SLIM using UniFFI.

## Prerequisites

- Rust toolchain
- Node.js >= 18
- [Task](https://taskfile.dev/)

## Usage

### 1. Generate Bindings

```bash
task generate
```

### 2. Run P2P Examples

```bash
# Terminal 1: Start the server
task example:server

# Terminal 2: Start Alice (receiver)
task example:alice

# Terminal 3: Start Bob (sender) 
task example:bob
```

### Available Commands

```bash
task generate         # Generate bindings
task clean            # Clean build artifacts
task example:server   # Run server
task example:alice    # Run Alice receiver
task example:bob      # Run Bob sender
```


## Resources

- [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node)
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/)
- [ffi-rs](https://www.npmjs.com/package/ffi-rs)
- [SLIM Core](https://github.com/agntcy/slim)
