# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-proto` rather than fuzzing
for its own sake.

- **`message-decode`** — decodes arbitrary bytes as a dataplane `Message`,
  the same operation the WebSocket read loop performs on every binary frame
  (`crates/datapath/src/websocket/stream.rs`, `OpCode::Binary`). The payload
  is entirely peer-controlled and unauthenticated at that point, so a panic
  here would take down the read loop of a live connection rather than
  returning `Status::invalid_argument`. Also asserts that anything which
  decodes survives a re-encode and decode unchanged — prost round-trips are
  not byte-identical, so the assertion is on the decoded value, not the bytes.

- **`parse-name`** — parses arbitrary strings with `Name::parse_name`, the
  only thing standing between an arbitrary string and a SLIM name. Names
  arrive from config files, `slimctl` arguments and peer control messages.
  Beyond "does not panic", this asserts a parsed name is *complete*: both
  `str_components` and `Display` `expect(...)` the string form to be present,
  so a successful parse that left a component empty or unset would be a panic
  waiting to happen in a caller.

## Running

From the repo root:

```bash
task fuzz:list                                  # show targets
task fuzz                                       # message-decode, 60s
task fuzz FUZZ_TARGET=parse-name SECS=300       # a specific target, longer
task fuzz:seeds                                 # regenerate seeds/
```

`task fuzz` installs `cargo-fuzz` and the nightly toolchain on first use.

The tasks select nightly by putting its bin directory first on `PATH`, not
with `cargo +nightly` or `RUSTUP_TOOLCHAIN`. Both of those are rustup
mechanisms and are silently ignored when `cargo` on `PATH` is not a rustup
proxy — a Homebrew `rust` install, for example. `cargo-fuzz` also spawns a
plain `cargo` of its own, which has to resolve to nightly too.

For reference, both targets manage roughly 3M runs/minute on an M-series
laptop, so a 60s default is a meaningful smoke test rather than a token one.

## Why this is a separate crate

libFuzzer requires nightly. The `[workspace]` stanza at the bottom of
`Cargo.toml` detaches this crate from the repo root, so `cargo build
--workspace` never sees it and the nightly requirement stays in this
directory. The repo toolchain stays pinned to stable in
`rust-toolchain.toml`.

## Adding a target

1. Add `fuzz_targets/<name>.rs` with a module doc comment naming the boundary
   it mirrors and the invariant it asserts.
2. Add a `[[bin]]` entry to `Cargo.toml`.
3. Add a bullet above, in the same form: what it parses, which real code path
   it stands in for, and why a panic there would matter.

Reaching a `pub(crate)` decoder needs either widened visibility or a target
inside the crate; prefer a `fuzzing` feature gate over a blanket `pub` so the
widened surface does not become de facto public API.

## Seeds

`seeds/<target>/` holds read-only starting inputs, passed alongside the
writable `corpus/<target>/` on every run. Coverage-guided fuzzing from an
empty corpus spends most of a short budget rediscovering basic structure —
valid protobuf tags and wire types, or the `a/b/c` shape of a name — which
matters most for a time-boxed CI run.

They are generated rather than hand-written, so they stay valid as the schema
moves: `examples/gen-seeds.rs`, run via `task fuzz:seeds`. It is an example
and not a `[[bin]]` because `cargo fuzz` builds `--bins`, and a binary without
`fuzz_target!` is not a valid fuzz target.

## CI

`.github/workflows/fuzz.yaml` runs each target in its own matrix job: 60s on
pull requests as a smoke test, 300s on the nightly cron. Crash inputs upload
as artifacts on failure — without them the log says only that something
failed.

It is deliberately **not** a required status check. It is path-filtered, and a
required check that does not report on every pull request leaves those PRs
permanently pending.

## Not yet covered

Tracked separately, all building on this scaffolding:

- MLS message processing (`process_commit` / `welcome` / `proposal`) — the
  highest-value surface, since those take bytes straight from remote peers
- session persistence decoders and the at-rest cipher
- datapath header MAC verification and link key exchange
- config and `slimctl` string parsers
- seed corpora and a time-boxed CI job

Seed corpora are deliberately absent for now: coverage-guided fuzzing from an
empty corpus spends most of a short budget rediscovering basic structure, so
seeds matter most once this runs on a clock in CI.
