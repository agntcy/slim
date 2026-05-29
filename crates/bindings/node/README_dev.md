# SLIM JavaScript / TypeScript bindings (`@agntcy/slim-bindings`) — developer guide

This is the npm package for using SLIM from JavaScript and TypeScript. It targets **Node.js** (≥18): servers, services, CLIs, tests, and any workflow where the native addon loads through Node.

Bindings are generated with [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node) and talk to the Rust core via [ffi-rs](https://www.npmjs.com/package/ffi-rs).

For **using the package** (install, quick start, main API patterns), see the root [README.md](./README.md).

## Features

- **Primary JS/TS entry point** — Install `@agntcy/slim-bindings` for Node; optional `@agntcy/slim-bindings-*` packages supply the correct native binary per OS/arch.
- **Rust-backed core** — Same SLIM logic as other language bindings; native code ships per platform.
- **TypeScript** — Typed surface and editor support (with known FFI quirks documented below).
- **Authentication** — Shared secret, JWT, and SPIRE-oriented flows exposed through the generated API.
- **Async-first** — Promise-based calls aligned with Node conventions.
- **UniFFI** — Generated from Mozilla’s [UniFFI](https://mozilla.github.io/uniffi-rs/) bindings.

## Installation

Published builds install from npm only (no local Rust required):

```bash
npm install @agntcy/slim-bindings
```

That pulls this package plus the matching optional native addon for your platform when a release exists for your version.

To build from source (development or an unpublished platform), see [Build from source](#build-from-source).

## Prerequisites (build from source)

- Rust toolchain
- Node.js >= 18
- [Task](https://taskfile.dev/)

## Usage (development workflow)

### 1. Generate bindings (from source)

```bash
task generate
```

### 2. Run P2P examples

```bash
# Terminal 1: Start the server
task example:server

# Terminal 2: Start Alice (receiver)
task example:alice

# Terminal 3: Start Bob (sender)
task example:bob
```

### Available commands

```bash
task generate         # Generate bindings
task clean            # Clean build artifacts
task example:server   # Run server
task example:alice    # Run Alice receiver
task example:bob      # Run Bob sender
```

## Build process

Generation applies small patches so output from `uniffi-bindgen-node` works with the shared `uniffi-bindgen-react-native` runtime dependency:

- **Naming**: `FfiConverterBytes` → `FfiConverterArrayBuffer`
- **Buffers**: Correct RustBuffer handling for byte arrays
- **Pointers**: BigInt ↔ Number at FFI boundaries where needed
- **Errors**: Clearer extraction from Rust error types

## FFI type conversions

Generated TypeScript may use `bigint` for some unsigned 64-bit values while the FFI layer returns JavaScript `number` at runtime. Normalize at your API boundary:

```typescript
const connId = await service.connectAsync(config);
await app.subscribeAsync(name, BigInt(connId));
app.setRoute(name, Number(connId));
```

- `connectAsync` often behaves like `number` at runtime → use `BigInt()` when a parameter expects `bigint`.
- When calling through to FFI with u64 parameters → use `Number()` from `bigint` when required.

## Build from source

1. Install Rust and Task.
2. Run `task generate` (builds the Rust library and emits Node bindings).
3. Consume files under `generated/` or run examples (`task example:server`, etc.).

Optional platform packages (`@agntcy/slim-bindings-*`) are version-pinned beside this package in `package.json`. For a **local** platform tarball from `task pack:platform`, install it explicitly (for example `npm install ./dist/node-darwin-arm64.tgz`) in addition to this package.

## Publishing (maintainers)

- **Dry run**: From `data-plane/bindings/node`, run `npm pack` for the main tarball; platform bundles via `task pack:platform TARGET=<target>` → `dist/node-<platform>.tgz`.
- **Version**: In `package.json`; release tags use `slim-bindings-v*` (see `.github/scripts/get-binding-version.sh`).
- **CI**: Releases publish via [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/) (OIDC from `release-bindings.yaml`); no publish token in GitHub. For **local** `npm publish`, use `npm login` or an automation token as usual.

## Resources

- [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node)
- [UniFFI](https://mozilla.github.io/uniffi-rs/)
- [ffi-rs](https://www.npmjs.com/package/ffi-rs)
- [SLIM](https://github.com/agntcy/slim)
- [React Native bindings](../react-native/README.md)
