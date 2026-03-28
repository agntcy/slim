# SLIM WASM vs Native — Gap Analysis

> Generated from a depth comparison of every public API in the native and WASM implementations.

## Executive Summary

The WASM client (`slim-wasm`) has **strong parity for the client-side workflow**: connect → subscribe → create sessions (P2P + multicast) → publish → receive → MLS encryption. The gaps fall into two categories:

- **Architectural** — requires native-only infrastructure (servers, gRPC, OS sockets). These are inherent browser limitations.
- **Fixable** — the underlying Rust types already compile for `wasm32-unknown-unknown`; they just aren't wired into the JS API yet.

---

## 1. Service Layer

| Feature | Native | WASM | Gap Type |
|---------|--------|------|----------|
| `Service` struct (server + multi-conn manager) | ✅ Full | ❌ Absent | Architectural |
| Accept inbound connections (gRPC/WS server) | ✅ | ❌ | Architectural |
| Multiple simultaneous connections | ✅ `HashMap<endpoint, conn_id>` | ❌ Single WS | **Fixable** |
| `get_connection_id(endpoint)` | ✅ | ❌ | **Fixable** |
| `get_all_connections()` → `Vec<ConnectionInfo>` | ✅ | ❌ | **Fixable** |
| `ServiceConfiguration` / YAML config loading | ✅ `ConfigLoader::new(file)` | ❌ | **Fixable** (accept JS object) |
| `run()` / `shutdown()` lifecycle | ✅ | ❌ (connect/disconnect only) | Architectural |

---

## 2. App Layer (Subscribe, Route, Session)

| Feature | Native (`App`) | WASM (`SlimClient`) | Gap Type |
|---------|---------------|---------------------|----------|
| `subscribe(name, conn)` | ✅ with conn scope | ✅ no conn param | **Fixable** — conn param omitted (single-conn) |
| `unsubscribe(name, conn)` | ✅ with conn scope | ✅ no conn param | **Fixable** |
| `set_route(name, conn)` | ✅ | ❌ Not exposed | **Fixable** |
| `remove_route(name, conn)` | ✅ | ❌ Not exposed | **Fixable** |
| `create_session(config, dest, id)` | ✅ Full `SessionConfig` | ✅ Hardcoded retries=10, interval=2s | **Fixable** — accept config from JS |
| `create_session` with explicit `Direction` | ✅ via `create_app_with_direction` | ❌ Hardcoded `Bidirectional` | **Fixable** |
| `create_session` with explicit session ID | ✅ `id: Option<u32>` | ❌ Always random | **Fixable** |
| `delete_session` → `CompletionHandle` | ✅ Awaitable | ✅ Fire-and-forget | **Fixable** |
| `clear_all_sessions()` | ✅ Returns map of handles | ❌ Not exposed | **Fixable** |
| `app_name()` | ✅ `&Name` | ✅ `String` (via getter) | Cosmetic |
| Custom `SessionConfig.metadata` | ✅ | ❌ Always empty | **Fixable** |
| Subscription ACK tracking | ✅ `oneshot` per sub | ✅ Same pattern | None |

---

## 3. Session Layer

The session layer is **fully cross-platform** via `runtime.rs` abstractions. All `SessionLayer` and `SessionController` methods compile and run on both targets.

### Behavioral Differences

| Aspect | Native | WASM | Impact |
|--------|--------|------|--------|
| `spawn` | `tokio::spawn` (requires `Send + 'static`) | `spawn_local` (no `Send`) | WASM tasks can capture non-Send types |
| Bounded channels | Real `tokio::sync::mpsc::channel(N)` | Unbounded `futures_channel` wrapped as bounded | **No backpressure on WASM** — can OOM under very high load |
| `JoinHandle::await` | Returns task result | Always `Pending` (fire-and-forget) | `CompletionHandle` works via oneshot; `JoinHandle`-based handles don't |
| `CancellationToken` | `tokio_util` (optimized, tree structure) | Custom `AtomicBool` + waker list | Functionally equivalent |
| `sleep` | `tokio::time::sleep` (monotonic) | `gloo_timers::TimeoutFuture` (browser `setTimeout`) | Timer precision differs; browser may throttle background tabs |
| `select!` | `tokio::select!` macro | Manual `poll_fn` combining futures | Functionally equivalent |
| `Deadline` (resettable timer) | `tokio::time::Sleep` (resetable) | `Instant::now() + duration` with gloo timer polling | Functionally equivalent |

### Exposed in WASM SlimClient vs available on SessionController

| SessionController Method | Exposed in WASM? | Notes |
|--------------------------|-------------------|-------|
| `publish(name, blob, type, meta)` | ✅ | |
| `publish_to(name, forward_to, blob, type, meta)` | ❌ | **Fixable** |
| `publish_with_flags(name, flags, blob, type, meta)` | ❌ | **Fixable** |
| `publish_message(raw_msg)` | ❌ | **Fixable** |
| `invite_participant(dest)` | ✅ | |
| `remove_participant(dest)` | ✅ | |
| `participants_list()` | ✅ | |
| `close()` → `JoinHandle` | ❌ | **Fixable** |
| `on_message_from_app(msg)` | ❌ (internal) | Internal |
| `id()`, `source()`, `dst()`, `session_type()` | ❌ (not exposed to JS) | **Fixable** — add getters |
| `metadata()`, `session_config()`, `is_initiator()` | ❌ (not exposed to JS) | **Fixable** — add getters |

---

## 4. MLS (Messaging Layer Security)

| Feature | Native | WASM | Gap |
|---------|--------|------|-----|
| Crypto provider | `mls-rs-crypto-awslc` (AWS-LC) | `mls-rs-crypto-webcrypto` (SubtleCrypto) | None — auto-swapped |
| `Mls::new()`, `initialize()` | ✅ | ✅ | None |
| `create_group()` | ✅ | ✅ | None |
| `generate_key_package()` | ✅ | ✅ | None |
| `add_member()` / `remove_member()` | ✅ | ✅ | None |
| `encrypt_message()` / `decrypt_message()` | ✅ | ✅ | None |
| `process_commit()` / `process_welcome()` | ✅ | ✅ | None |
| `create_rotation_proposal()` | ✅ | ✅ | None |
| MLS session interceptor | ✅ | ✅ (via session layer) | None |
| `mls_enabled` flag in `createSession` | ✅ | ✅ | None |

**MLS has full parity.** The WebCrypto provider is a first-class citizen.

---

## 5. Auth Layer

| Auth Provider | Native | WASM | Gap Type |
|---------------|--------|------|----------|
| `SharedSecret` (HMAC-SHA256) | ✅ `aws-lc-rs` | ✅ `hmac` + `sha2` crates | None |
| `TokenProvider` / `Verifier` traits | ✅ `async_trait` | ✅ `async_trait(?Send)` | None |
| JWT (sign/verify) | ✅ `jsonwebtoken-aws-lc` | ❌ | **Fixable** — use `jsonwebtoken` with ring/wasm |
| JWT tower middleware | ✅ | ❌ | Architectural (server-side) |
| OIDC (OAuth2) | ✅ `oauth2` + `reqwest` | ❌ | **Partially fixable** — needs wasm HTTP client |
| SPIRE (SPIFFE) | ✅ Unix socket | ❌ | Architectural |
| BasicAuth config | ✅ | ✅ (struct only) | None |
| Auth provider factory/builder | ✅ | ❌ | **Fixable** |
| File watcher (cert rotation) | ✅ `notify` crate | ❌ | Architectural |
| MetadataMap | ✅ | ✅ | None |

---

## 6. Config & Transport

| Feature | Native | WASM | Gap Type |
|---------|--------|------|----------|
| gRPC transport (HTTP/2) | ✅ Client + Server | ❌ | Architectural |
| WebSocket transport | ✅ Client + Server | ✅ Client only | Architectural (server) |
| TLS (rustls) | ✅ Full config | ❌ (browser handles via `wss://`) | Architectural |
| mTLS (mutual TLS) | ✅ | ❌ | Architectural |
| Unix sockets | ✅ | ❌ | Architectural |
| YAML config loading | ✅ `ConfigLoader` | ❌ | **Fixable** |
| Proxy support | ✅ HTTP proxy config | ❌ (browser controls proxies) | Architectural |
| Backoff strategies | ✅ exponential/fixed | ✅ (structs available, not wired) | **Fixable** |
| WebSocket auth handshake | ✅ HTTP header | ✅ Query parameter | Different mechanism (browser limitation) |
| Compression (gRPC) | ✅ gzip/zstd | ❌ | Architectural |

---

## 7. Datapath Layer

| Module | Native | WASM | Gap Type |
|--------|--------|------|----------|
| Proto message types (`ProtoMessage`, `Content`, etc.) | ✅ | ✅ | None |
| `ProtoMessage::builder()` | ✅ | ✅ | None |
| `Name` (encoding, hashing, matching) | ✅ | ✅ | None |
| Forwarding tables | ✅ | ✅ | None |
| Message utilities (`SlimHeaderFlags`, constants) | ✅ | ✅ | None |
| `MessageProcessor` (routing engine) | ✅ | ❌ | Architectural |
| `Forwarder` | ✅ | ❌ | Architectural |
| Connection tracking | ✅ | ❌ | Architectural |
| WebSocket codec (server-side) | ✅ | ❌ | Architectural |
| gRPC service stubs (tonic) | ✅ | ❌ | Architectural |
| `Status` type | `tonic::Status` | Lightweight shim `{ code, message }` | Cosmetic |

---

## 8. Connection Management & Reliability

| Feature | Native | WASM | Gap Type |
|---------|--------|------|----------|
| Multi-connection support | ✅ | ❌ Single WS | **Fixable** |
| `conn_id` tracking | ✅ Per-connection u64 | ❌ Hardcoded to 0 | **Fixable** |
| Auto-reconnect on disconnect | ❌ Not implemented | ❌ Not implemented | **Fixable** (neither has it) |
| Transport-level keepalive | ✅ Configurable WS keepalive | ❌ Browser-controlled | Architectural |
| Session-level ping (10s interval) | ✅ | ✅ (shared session code) | None |
| Participant disconnect detection | ✅ After 3 missed pings | ✅ Same | None |
| Graceful shutdown / drain | ✅ `StartDrain { grace_period }` | ✅ (session layer shared) | None |
| Error propagation to app | ✅ `SessionError::SlimChannelClosed` | ✅ Same via notification loop | None |

---

## 9. Tracing & Observability

| Feature | Native | WASM | Gap Type |
|---------|--------|------|----------|
| Console logging | ✅ stdout/stderr | ✅ `web_sys::console::log` | None |
| Log level configuration | ✅ | ✅ | None |
| Per-module log filters | ✅ | ✅ (same defaults) | None |
| OpenTelemetry traces (OTLP/gRPC) | ✅ | ❌ | **Partially fixable** (OTLP/HTTP) |
| OpenTelemetry metrics | ✅ | ❌ | **Partially fixable** (OTLP/HTTP) |
| Prometheus metrics endpoint | ✅ | ❌ | Architectural (server) |
| Instance ID from env var | ✅ `SLIM_INSTANCE_ID` | ❌ UUID only | **Fixable** |
| Structured JSON output | ✅ | ❌ | **Fixable** |

---

## 10. JS API Surface Gaps (WASM-specific)

These are things the WASM client *could* expose but doesn't:

| Missing JS API | Available internally? | Difficulty |
|----------------|----------------------|------------|
| `publishTo(sessionId, name, forwardTo, payload)` | ✅ `SessionController::publish_to` | Easy |
| `publishWithFlags(sessionId, name, flags, payload)` | ✅ `SessionController::publish_with_flags` | Easy |
| `setRoute(org, ns, name, connId)` | ✅ Same message pattern as subscribe | Easy |
| `removeRoute(org, ns, name, connId)` | ✅ Same message pattern as unsubscribe | Easy |
| `clearAllSessions()` | ✅ `SessionLayer::clear_all_sessions` | Easy |
| `getSessionInfo(sessionId)` → `{ id, source, dest, type, initiator }` | ✅ All getters exist on `SessionController` | Easy |
| `createSession` with full config (retries, interval, metadata, session ID) | ✅ `SessionConfig` compiles for WASM | Medium |
| `createSession` with `Direction` | ✅ `Direction` is platform-agnostic | Easy |
| `publishRaw(sessionId, protoMessage)` — send a pre-built protobuf | ✅ `SessionController::publish_message` | Medium |
| `onDisconnect` callback | ❌ Need to detect WS close | Medium |
| `reconnect()` | ❌ Not implemented on either platform | Medium |
| Multiple WS connections | ❌ Needs new architecture | Hard |

---

## Summary: Gap Counts

| Category | Gaps | Fixable | Architectural |
|----------|------|---------|---------------|
| Service layer | 7 | 4 | 3 |
| App layer | 9 | 9 | 0 |
| Session layer (behavior) | 3 | 0 | 3 (inherent) |
| Session API exposure | 8 | 8 | 0 |
| MLS | 0 | — | — |
| Auth | 5 | 3 | 2 |
| Config/Transport | 8 | 2 | 6 |
| Datapath | 5 | 0 | 5 |
| Connection mgmt | 3 | 2 | 1 |
| Tracing | 4 | 3 | 1 |
| **Total** | **52** | **31** | **21** |

**31 gaps are fixable** without any architectural changes — the Rust types already compile for `wasm32-unknown-unknown` and just need to be wired into the `SlimClient` JS API.

**21 gaps are architectural** — they require native-only infrastructure (OS sockets, tokio runtime, tonic gRPC, TLS stack, server listeners) that fundamentally cannot run in a browser sandbox.
