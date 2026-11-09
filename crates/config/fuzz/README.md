# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-config` rather than
fuzzing for its own sake. Same scaffolding as `crates/proto/fuzz` (see that
crate's README for the general rationale), scoped to this crate instead.

- **`websocket-endpoint-parse`** — parses arbitrary strings with
  `WebSocketEndpoint::parse` (`crates/config/src/websocket/common.rs`), which
  reads the `endpoint` of every WebSocket client and server config. Its result
  is used three ways: the server binds `socket_address()`
  (`websocket/server.rs`), the native client drives the upgrade with
  `http_request_uri()` (`websocket/client.rs`), and the browser client
  connects with `request_uri()` (`websocket/client_wasm.rs`). A parse that
  succeeds but hands any of those something unusable turns a config mistake
  into a confusing bind or connect failure at runtime, so beyond "does not
  panic" the target asserts that an accepted endpoint:

  - is `ws` or `wss`, has a non-empty host, a path starting with `/`, and
    either its explicit port or the scheme's default (80 / 443);
  - yields a `socket_address()` that parses as a socket address when the
    host is an IP literal;
  - can build both request URIs, which keep the authority, path and query,
    and parse back to the same endpoint.

  Its first run found that `http::Uri` accepts authorities with no host
  (`ws://:80/`, `ws://user@:80/`, `ws://[]:80/`); `parse` now rejects them.

## Running

From this directory (nightly on `PATH`, not `cargo +nightly` — see
`crates/proto/fuzz/README.md`'s "Running" section for why):

```bash
cargo fuzz list
cargo fuzz run websocket-endpoint-parse -- -max_total_time=60
```

`task fuzz`/`task fuzz:list`/`task fuzz:build` are not wired up for this crate
yet — they are currently hardcoded to `crates/proto/fuzz`, and extending them
to cover every fuzz crate is being done separately to avoid multiple PRs
touching the same `Taskfile.yaml`/`fuzz.yaml` at once.

## Why this is a separate crate

libFuzzer requires nightly. The `[workspace]` stanza at the bottom of
`Cargo.toml` detaches this crate from the repo root, so `cargo build
--workspace` never sees it and the nightly requirement stays in this
directory. The repo toolchain stays pinned to stable in `rust-toolchain.toml`.

`Cargo.lock` starts from the root lockfile so the build scripts this crate
pulls in (`tonic-prost-build` among them) match the repo's versions; a newer
one rewrites the checked-in generated sources in `crates/proto/src/gen/` and
`crates/config/src/testutils/` during a fuzz build.

## Visibility

`WebSocketEndpoint` is already public as
`slim_config::websocket::common::WebSocketEndpoint`, so no feature gate or
widened visibility is needed.

## Not yet covered

- Seed corpora — see `crates/proto/fuzz/README.md`
- CI wiring (`.github/workflows/fuzz.yaml`, `Taskfile.yaml`) — tracked
  separately, alongside the other per-crate fuzz crates
- The rest of the config surface (gRPC client/server configs, TLS and auth
  settings); this target only covers the WebSocket endpoint string
