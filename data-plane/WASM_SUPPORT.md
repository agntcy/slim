# WASM/Browser Support Status

## ✅ Completed
1. **Config WebSocket-WASM Feature**: The `agntcy-slim-config` crate now compiles for `wasm32-unknown-unknown` with the `websocket-wasm` feature enabled
   - Uses `gloo-net` for browser-compatible WebSocket/HTTP client
   - Command: `cargo check -p agntcy-slim-config --target wasm32-unknown-unknown --no-default-features --features websocket-wasm`

2. **MLS Crypto Provider Abstraction**: Created pluggable crypto provider in `core/mls/src/crypto.rs`
   - `native` feature (default): Uses `mls-rs-crypto-awslc` (AWS-LC)
   - `wasm` feature: Uses `mls-rs-crypto-webcrypto` (SubtleCrypto for browser)
   - Type alias `CryptoProviderImpl` switches automatically based on feature

## ⚠️ Current Blockers for WASM

### Tokio/Mio Issue
The MLS crate cannot compile for WASM because of transitive dependencies:
```
MLS → Auth → Tokio + tokio-util + other deps
MLS → Datapath → Tokio features
```

These pull in `mio` which has this error on WASM:
```
error: This wasm target is unsupported by mio. If using Tokio, disable the net feature.
```

## 🛣️ Path to Full WASM Support

### Option 1: Feature-Gate Tokio in Dependencies (Recommended)
1. Make `tokio` optional in `core/auth/Cargo.toml`
2. Make `tokio` optional in `core/datapath/Cargo.toml`  
3. Use feature flags like `#[cfg(feature = "runtime-tokio")]` to conditionally compile tokio-dependent code
4. Create minimal/alternative implementations for WASM (e.g., simple trait implementations that don't need async runtimes)

### Option 2: Separate WASM Build
1. Create a minimal `slim-wasm` library that doesn't depend on auth/datapath
2. Implement trait stubs that work in-browser
3. Use `no_std` + `wasm-bindgen` for a lighter WASM build

### Option 3: Use MLS Only When Not WASM
1. Keep session/service crates WASM-optional for MLS
2. Build config + crypto only for WASM initially
3. Phase in full MLS support later

## Testing WASM Builds

```bash
# WebSocket client only (works today)
cd data-plane
cargo check -p agntcy-slim-config --target wasm32-unknown-unknown --no-default-features --features websocket-wasm

# Native build (works with full features)
cargo check -p agntcy-slim-mls --features native

# MLS WASM (would work after fixing dependency blocker)
cargo check -p agntcy-slim-mls --target wasm32-unknown-unknown --no-default-features --features wasm
```

## Browser Runtime Architecture

For running data-plane in the browser:
1. **Communication**: ✅ WebSocket support ready (`websocket-wasm` feature)
2. **Crypto**: ✅ MLS WebCrypto provider available (needs tokio fix)
3. **Session Management**: ⏳ Requires tokio removal from dependencies
4. **Auth**: ⏳ Needs simplified WASM-compatible implementation

The websocket-wasm foundation is ready today. Full data-plane browser support requires solving the tokio/mio incompatibility.
