# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-persistence` rather than
fuzzing for its own sake. Same scaffolding as `crates/proto/fuzz` (see that
crate's README for the general rationale), scoped to this crate instead.

- **`decrypt-tamper`** — decrypts arbitrary and deliberately-tampered blobs
  with `ValueCipher::decrypt` (`crates/persistence/src/cipher.rs`), the value-
  level AES-256-GCM decryption every read from the at-rest SQLite store passes
  through before the rest of the code sees plaintext. The database engine
  itself is plain (unencrypted), so `decrypt` is the only thing standing
  between a corrupted-on-disk or actively-tampered blob and being treated as
  trusted plaintext — a panic here would take down a store read, and a
  successful decrypt of a modified blob would be a silent authentication
  bypass, either of which matters far more than most fuzz targets' "just don't
  panic".

  Fully random input trivially satisfies "AEAD rejects it" (almost nothing
  coincidentally passes GCM authentication), so the target does not stop
  there: it first encrypts a known plaintext with a real derived key, then
  uses the fuzzer input to choose a mutation — truncate, extend, flip one bit
  anywhere, or flip bits only within the 12-byte nonce prefix — and applies it
  to that real ciphertext before decrypting. `decrypt` returning `Ok` for
  anything that isn't byte-identical to the original blob would be a
  truncation/nonce-manipulation bypass of AEAD authentication.

## Running

From this directory (nightly on `PATH`, not `cargo +nightly` — see
`crates/proto/fuzz/README.md`'s "Running" section for why):

```bash
cargo fuzz list
cargo fuzz run decrypt-tamper -- -max_total_time=60
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

`ValueCipher` and its `encrypt`/`decrypt` methods are `pub(crate)` — at-rest
encryption is an internal detail of the store, not a surface anything outside
this crate is meant to call. Widening them to plain `pub` would make the raw
AEAD operations de facto public API forever, just to let a fuzz harness reach
them, and would also require exposing the `ValueCipher` type itself.

Instead, `agntcy-slim-persistence` gained a `fuzzing` Cargo feature (off by
default, not part of any default feature set — see `[features]` in
`crates/persistence/Cargo.toml`) that gates two free functions in
`cipher.rs`, `encrypt_fuzz`/`decrypt_fuzz`, each `#[doc(hidden)] pub`. Their
signatures only use already-public types (`&str`, `&[u8]`,
`Result<Vec<u8>, PersistenceError>`), so `ValueCipher` never has to be exposed
at all — each just derives a `ValueCipher` internally from a fixed test
identity and calls the real method. They're re-exported as
`slim_persistence::fuzzing::{encrypt, decrypt}` (see the `fuzzing` module in
`crates/persistence/src/lib.rs`). This crate is the only place in the repo
that enables the feature
(`crates/persistence/fuzz/Cargo.toml`'s `[dependencies.agntcy-slim-persistence]`).
`#[doc(hidden)]` keeps the surface out of rustdoc even for a downstream crate
that turns the feature on by mistake, and the feature being off by default
means an ordinary `cargo build`/`cargo build --workspace` of this crate never
exposes it at all.

## Not yet covered

- Seed corpora (coverage-guided fuzzing from an empty corpus spends most of a
  short budget rediscovering basic structure, so seeds matter most once this
  runs on a clock in CI — see `crates/proto/fuzz/README.md`)
- CI wiring (`.github/workflows/fuzz.yaml`, `Taskfile.yaml`) — tracked
  separately, alongside the same wiring for `crates/session/fuzz`
- The MLS group-state storage path (`group_storage.rs`) and the SQLite
  backend itself — this target only covers the value cipher
