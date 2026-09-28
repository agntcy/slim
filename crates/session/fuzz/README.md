# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-session` rather than
fuzzing for its own sake. Same scaffolding as `crates/proto/fuzz` (see that
crate's README for the general rationale), scoped to this crate instead.

- **`decode-name`** — decodes arbitrary bytes as a persisted `ProtoName` via
  `decode_name` (`crates/session/src/persistence.rs`). Session records store
  their `source`/`destination`/`control` names, and a moderator's
  `mls_participants` map, as prost-encoded bytes inside an otherwise
  serde/JSON record; `decode_name` is what turns that `Vec<u8>` back into a
  name on restore. That bytes came back out of the KV store rather than being
  freshly produced by this process, so a corrupted or tampered record is
  exactly what this decoder sees first — a panic here would take down session
  restore instead of `PersistedSession::from_bytes`/restore cleanly reporting
  `SessionError::PersistenceDecode`. Also asserts that a name built from
  `Arbitrary`-generated components survives an encode/decode round trip
  unchanged.

- **`decode-participant`** — the same boundary and invariant as
  `decode-name`, for `decode_participant` and each entry of a session
  record's `group_list` (a `Participant`, which itself carries a `ProtoName`).

Both targets also cover the "untrusted length prefix" concern directly: they
feed fully arbitrary bytes into the decoder (most of which aren't a valid
encoding at all) and rely on the fuzzer's default OOM/leak detection to catch
a runaway allocation, on top of the no-panic assertion. See "Not a bug" below
for why this is expected to (and does) pass clean.

## Running

From this directory (nightly on `PATH`, not `cargo +nightly` — see
`crates/proto/fuzz/README.md`'s "Running" section for why):

```bash
cargo fuzz list
cargo fuzz run decode-name -- -max_total_time=60
cargo fuzz run decode-participant -- -max_total_time=60
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

## Visibility: `fuzzing` feature, not a blanket `pub`

`decode_name` and `decode_participant` are `pub(crate)` — they are an internal
parsing step of the persistence module, not a surface anything outside this
crate is meant to call. Widening them to plain `pub` would make that de facto
public API forever, just to let a fuzz harness reach them.

Instead, `agntcy-slim-session` gained a `fuzzing` Cargo feature (off by
default, not part of any default feature set — see `[features]` in
`crates/session/Cargo.toml`) that gates a `#[doc(hidden)] pub` wrapper around
each function (`decode_name_fuzz`/`decode_participant_fuzz` in
`persistence.rs`), re-exported as `agntcy_slim_session::fuzzing::{decode_name,
decode_participant}` (see the `fuzzing` module in `crates/session/src/lib.rs`).
This crate is the only place in the repo that enables the feature
(`crates/session/fuzz/Cargo.toml`'s `[dependencies.agntcy-slim-session]`).
`#[doc(hidden)]` keeps the surface out of rustdoc even for a downstream crate
that turns the feature on by mistake, and the feature being off by default
means an ordinary `cargo build`/`cargo build --workspace` of this crate never
exposes it at all.

## Not yet covered

- Seed corpora (coverage-guided fuzzing from an empty corpus spends most of a
  short budget rediscovering basic structure, so seeds matter most once this
  runs on a clock in CI — see `crates/proto/fuzz/README.md`)
- CI wiring (`.github/workflows/fuzz.yaml`, `Taskfile.yaml`) — tracked
  separately, alongside the same wiring for `crates/persistence/fuzz`
