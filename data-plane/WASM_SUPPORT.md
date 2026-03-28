# WASM/Browser Support Status

## ✅ Completed
1. **Config WebSocket-WASM Feature**: The `agntcy-slim-config` crate now compiles for `wasm32-unknown-unknown` with the `websocket-wasm` feature enabled
   - Uses `gloo-net` for browser-compatible WebSocket/HTTP client
   - Command: `cargo check -p agntcy-slim-config --target wasm32-unknown-unknown --no-default-features --features websocket-wasm`

2. **MLS Crypto Provider Abstraction**: Created pluggable crypto provider in `core/mls/src/crypto.rs`
   - `native` feature (default): Uses `mls-rs-crypto-awslc` (AWS-LC)
   - `wasm` feature: Uses `mls-rs-crypto-webcrypto` (SubtleCrypto for browser)
   - Type alias `CryptoProviderImpl` switches automatically based on feature

3. **MLS Identity Provider Send Fix**: Fixed `Send` bound mismatch in `core/mls/src/identity_provider.rs`
   - The upstream `mls-rs-core::IdentityProvider` trait requires `Send` futures
   - On WASM, `Verifier::get_claims()` returns `!Send` futures via `async_trait(?Send)`
   - Fix: Gate the async `get_claims` fallback behind `#[cfg(feature = "native")]` — on WASM, `try_get_claims` (synchronous HMAC) always succeeds, so the async path is unreachable

4. **Controller Proto gRPC Gating**: Fixed `build.rs` module path (`controller.proto.v1` instead of `controller.v1`) so generated gRPC client/server stubs are properly gated behind `#[cfg(feature = "native")]`

5. **Full Crate WASM Compilation**: All data-plane crates now compile for `wasm32-unknown-unknown`:
   - `agntcy-slim-config` (websocket-wasm)
   - `agntcy-slim-auth` (wasm)
   - `agntcy-slim-datapath` (wasm)
   - `agntcy-slim-tracing` (wasm)
   - `agntcy-slim-signal` (wasm)
   - `agntcy-slim-mls` (wasm)
   - `agntcy-slim-session` (wasm)
   - `agntcy-slim-controller` (wasm)
   - `agntcy-slim-service` (wasm, session)

## Testing WASM Builds

```bash
cd data-plane

# Individual crate checks
cargo check -p agntcy-slim-config --target wasm32-unknown-unknown --no-default-features --features websocket-wasm
cargo check -p agntcy-slim-auth --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-datapath --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-tracing --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-signal --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-mls --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-session --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p agntcy-slim-controller --target wasm32-unknown-unknown --no-default-features --features wasm

# Full service (with session)
cargo check -p agntcy-slim-service --target wasm32-unknown-unknown --no-default-features --features wasm,session

# Native build (verify no regressions)
cargo check -p agntcy-slim-service --features native,session
```

## Browser Runtime Architecture

For running data-plane in the browser:
1. **Communication**: ✅ WebSocket support ready (`websocket-wasm` feature via gloo-net)
2. **Crypto**: ✅ MLS WebCrypto provider working (`mls-rs-crypto-webcrypto`)
3. **Session Management**: ✅ Platform abstraction via `core/session/src/runtime.rs` (futures-channel, gloo-timers, wasm-bindgen-futures)
4. **Auth**: ✅ HMAC-SHA256 via pure-Rust `hmac`+`sha2` crates
5. **MLS Identity**: ✅ Send bound fix applied, compiles for WASM
6. **Tracing**: ✅ Console-based fallback (no OpenTelemetry on WASM)
7. **Controller**: ✅ API types available, gRPC client/server gated behind native

All compilation blockers have been resolved. The full data-plane service compiles for `wasm32-unknown-unknown`.

## Known WASM Limitations

- **No WebSocket server**: Browser can only act as a WebSocket client, not accept connections
- **Auth headers**: Browser WebSocket API cannot set custom HTTP headers; use `websocket_auth_query_param` instead
- **Bounded channels**: WASM `channel(buffer)` ignores the buffer parameter (always unbounded)
- **JoinHandle**: WASM `spawn()` returns a fire-and-forget handle; awaiting it never resolves
- **Timer precision**: Durations > 1 day treated as infinite; capped at u32::MAX milliseconds (~49.7 days)
- **No gRPC**: tonic/HTTP2 server not available in browser; all communication via WebSocket

## Next Steps

1. Create a `wasm-bindgen` entry point exposing a JavaScript-callable API for SLIM session initialization
2. Add Taskfile entries for full WASM build checks
3. Add CI workflow for `wasm32-unknown-unknown` target verification
4. Test end-to-end browser ↔ native SLIM communication via WebSocket
