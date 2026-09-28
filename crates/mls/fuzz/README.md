# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-mls` rather than fuzzing
for its own sake. All three fuzz the message-processing entry points on
[`Mls`](../src/mls.rs) — `process_commit`, `process_welcome` and
`process_proposal` — which take bytes supplied directly by remote peers and
feed them into a live MLS group's state machine.

- **`process-commit`** — feeds arbitrary bytes to `Mls::process_commit`, what
  an already-joined group member does with every Commit broadcast by another
  member. The bytes are fully peer-controlled and unauthenticated until
  `Group::process_incoming_message` gets to decide, so a panic here would take
  down a live session rather than surface as a rejected commit. Beyond "does
  not panic", this asserts that a *rejected* commit leaves the group's epoch
  and group id exactly as they were — a bad actor must not be able to desync
  or half-apply state onto a live group merely by getting a message to
  `process_commit`.

- **`process-welcome`** — feeds arbitrary bytes to `Mls::process_welcome`,
  what a freshly-invited client does with the Welcome it receives from
  whoever added it. This is the first thing the client ever does with the
  group, so unlike `process-commit` there is no established state to check
  the sender against, and acceptance creates a brand-new group from scratch
  rather than advancing an existing one. Asserts that a *rejected* welcome
  does not create or otherwise change the client's group.

- **`process-proposal`** — feeds arbitrary bytes to `Mls::process_proposal`,
  what a group member does with a Proposal broadcast by another member (e.g.
  the credential-rotation proposals `create_rotation_proposal` produces).
  `process_proposal` also takes a `create_commit` flag chosen by the caller
  from network context rather than validated input, so the first fuzzed byte
  selects it and the rest is the proposal message. Asserts that a *rejected*
  proposal does not advance the epoch, that committing an accepted proposal
  advances the epoch by exactly one, and that staging one without committing
  advances nothing and returns no commit message.

## Native only

`mls-rs` (via `maybe-async`) is synchronous on native and asynchronous on
`wasm32`. These targets fuzz the native, synchronous shape only — `cargo fuzz`
runs as a native binary under libFuzzer, and fuzzing the async wasm32 shape
would need a browser or a wasm executor rather than plain `cargo fuzz`. Only
`Mls::initialize` (an always-async call used once during harness setup, not
part of the fuzzed input path) needs an executor at all; a minimal
current-thread `tokio` runtime handles that.

## Group setup: build once, reuse across iterations

Unlike the stateless parsers in `agntcy-slim-proto`'s fuzz crate, these
targets are stateful: each needs a live, already-joined MLS group before the
fuzzer's bytes are meaningful input at all. Building a fresh group on every
iteration would spend nearly the whole time budget on `mls-rs` group setup
(several asymmetric-crypto operations and a full add-member/welcome
handshake) rather than on the code being fuzzed.

Instead, each target builds its group **once**, behind a
`std::sync::OnceLock<std::sync::Mutex<_>>` (see `src/lib.rs`), and reuses it
for every iteration rather than cloning or resetting per input. This is sound
specifically because the invariant under test is that a *rejected* call
leaves the group unchanged: running many iterations against one persistent
group is exactly the scenario that invariant describes. An *accepted* input
is expected to legitimately advance the group and is left free to do so
across iterations — in practice this essentially never happens against
random bytes, since acceptance requires a validly-signed MLS structure.

This trade-off keeps throughput close to the stateless targets rather than
paying group setup on every call: a 30s local run measured ~3.2k-3.3k exec/s
per target (~100k total executions each) on an M-series laptop, all three
clean with no crashes.

## Running

From this directory (`crates/mls/fuzz`), with nightly on `PATH`:

```bash
PATH="$(dirname "$(rustup which --toolchain nightly cargo)"):$PATH"

cargo fuzz list                                        # show targets
cargo fuzz build                                        # build without running
cargo fuzz run process-commit -- -max_total_time=60     # a specific target
cargo fuzz run process-welcome -- -max_total_time=60
cargo fuzz run process-proposal -- -max_total_time=60
```

As in `crates/proto/fuzz`, nightly is selected by putting its bin directory
first on `PATH`, not with `cargo +nightly` or `RUSTUP_TOOLCHAIN` — both are
rustup mechanisms and are silently ignored when `cargo` on `PATH` is not a
rustup proxy (a Homebrew `rust` install, for example), and `cargo-fuzz` spawns
a plain `cargo` of its own that has to resolve to nightly too.

This crate is not yet wired into `task fuzz*` or `.github/workflows/fuzz.yaml`
— that wiring is being done separately once all the new fuzz crates from
issue #2074 exist, to avoid parallel PRs conflicting on those shared files.
Until then, run it locally with plain `cargo fuzz` as shown above.

## Why this is a separate crate

libFuzzer requires nightly. The `[workspace]` stanza at the bottom of
`Cargo.toml` detaches this crate from the repo root, so `cargo build
--workspace` never sees it and the nightly requirement stays in this
directory, exactly as `crates/proto/fuzz` does. The repo toolchain stays
pinned to stable in `rust-toolchain.toml`.

## Adding a target

1. Add `fuzz_targets/<name>.rs` with a module doc comment naming the boundary
   it mirrors and the invariant it asserts.
2. Add a `[[bin]]` entry to `Cargo.toml`.
3. Add a bullet above, in the same form: what it processes, which real code
   path it stands in for, and why a panic there would matter.
4. If it needs group scaffolding, reuse or extend `src/lib.rs` rather than
   duplicating setup in the target itself.

## Seeds

No seed corpus yet, for the same reason `crates/proto/fuzz` does not have one
for its newer targets: coverage-guided fuzzing from an empty corpus spends
most of a short budget rediscovering basic structure, so seeds matter most
once this runs on a clock in CI.
