# SLIM Node.js Bindings

Node.js bindings for SLIM using UniFFI.
Bindings generated with [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node).

## Installation

For released versions, install from npm (no Rust or build required):

```bash
npm install @agntcy/slim-bindings-node
```

This installs the main package and the matching platform-specific native addon for your OS/arch.

To build from source (e.g. for development or unsupported platforms), see [Build from source](#build-from-source) below.

## Prerequisites (build from source)

- Rust toolchain
- Node.js >= 18
- [Task](https://taskfile.dev/)

## Usage

### 1. Generate Bindings (build from source)

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

## Build Process

The bindings generation includes patching to fix compatibility issues between `uniffi-bindgen-node` (code generator) and `uniffi-bindgen-react-native` (runtime library):

- **Naming fixes**: `FfiConverterBytes` → `FfiConverterArrayBuffer`
- **Buffer wrapping**: Proper RustBuffer allocation for byte arrays
- **Pointer conversions**: BigInt → Number for FFI calls
- **Error handling**: Enhanced error extraction from Rust

## FFI Type Conversions

The generated bindings use `bigint` in TypeScript signatures for u64 types, but the underlying FFI layer returns JavaScript `number` types at runtime. Application code handles conversions at API boundaries:

```typescript
// connectAsync returns a number at runtime, despite bigint type signature
const connId = await service.connectAsync(config);

// Convert to BigInt when passing to methods expecting bigint
await app.subscribeAsync(name, BigInt(connId));

// When using with methods that need number (for direct FFI calls)
app.setRoute(name, Number(connId));
```

**Key conversions:**
- `connectAsync` returns `number` → convert to `BigInt()` for TypeScript bigint parameters
- When calling FFI methods with u64 params → convert to `Number()` if passing bigint

This explicit conversion at API boundaries ensures type safety and makes FFI requirements visible.

## Build from source

If you need to build from source (development or a platform not yet published):

1. Ensure Rust and Task are installed.
2. Run `task generate` (builds the Rust lib and generates Node bindings).
3. Use the bindings from `generated/` or run the examples with `task example:server`, etc.

## Publishing (maintainers)

- **Dry run**: From `data-plane/bindings/node`, run `npm pack` to produce a tarball and inspect contents (main package: no `generated/`; platform packages: `task pack:platform TARGET=<target>` then check `dist/node-<platform>.tgz`).
- **Version**: Set in package.json; CI derives version from tag `slim-bindings-v*` via `.github/scripts/get-binding-version.sh`.
- **Secrets**: `NPM_TOKEN` must be set in the repo for `npm publish`.

## Resources

- [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node)
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/)
- [ffi-rs](https://www.npmjs.com/package/ffi-rs)
- [SLIM Core](https://github.com/agntcy/slim)
