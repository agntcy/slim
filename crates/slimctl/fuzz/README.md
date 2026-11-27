# Fuzz targets

Each target mirrors a real boundary in `agntcy-slimctl` rather than fuzzing
for its own sake. Same scaffolding as `crates/proto/fuzz` (see that crate's
README for the general rationale), scoped to this crate instead. All three
take operator input from the command line or a config file.

- **`parse-duration`** — parses arbitrary strings with `parse_duration`
  (`crates/slimctl/src/config.rs`), which handles `--timeout` and
  `slimctl config set timeout`. The second one writes the value to the config
  file, which every later command parses again. Asserts that no input panics;
  that a value too large to represent is rejected rather than wrapped or
  saturated, by building inputs from `(number, unit)` pairs and checking the
  result against a total computed independently in `u128`; and that anything
  which parses survives being written back as a `DurationString` and parsed
  again, so `config set timeout` cannot leave an unreadable config file
  behind. Purely random bytes almost never produce a 20-digit number followed
  by a valid unit, which is why the overflow check uses structured input.

- **`parse-route`** — parses arbitrary strings with `parse_route`
  (`crates/slimctl/src/utils.rs`), which splits
  `org/namespace/agentname[/agentid]` into name components. Asserts that no
  input panics or indexes past the end, however many `/` separators it has,
  and that an accepted route has exactly two or three separators, three
  non-empty components that contain no `/` and match the input, and
  `NULL_COMPONENT` as the agent id when the fourth part is omitted.

- **`parse-config-file`** — `parse_config_file` takes a *path*, so fuzzing the
  path string would only ever exercise `open()` and "file not found". The
  target fuzzes the file **contents** instead, through `parse_config_data` and
  `parse_config_data_with_link_id`: the same parse-and-validate steps with the
  file read split out in front of them. Asserts that no input panics; that an
  accepted config is passed through unchanged as `config_data` with its
  endpoint as the connection's `link_id`; and that the `_with_link_id` variant
  always returns a non-empty link id inside a `config_data` that parses back
  to the same link id and endpoint, since the node registers the connection
  from that text.

## Running

From this directory (nightly on `PATH`, not `cargo +nightly` — see
`crates/proto/fuzz/README.md`'s "Running" section for why):

```bash
cargo fuzz list
cargo fuzz run parse-duration -- -max_total_time=60
cargo fuzz run parse-route -- -max_total_time=60 -max_len=65536
cargo fuzz run parse-config-file -- -max_total_time=60
```

`-max_len` raises libFuzzer's default 4096-byte input cap so very long
strings are tried too.

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

## Visibility: a library target

`slimctl` was binary-only, and a fuzz crate cannot depend on a binary.
`crates/slimctl/src/lib.rs` exposes the three modules the targets need —
`config`, `utils`, and `proto`, which `utils` uses — as the
`agntcy_slimctl` library. The `slimctl` binary keeps its own module tree and
does not depend on the library. `config` carries `#[allow(dead_code)]` in the
library because some of its `pub(crate)` items are used only by the binary.

## Not yet covered

- Seed corpora — see `crates/proto/fuzz/README.md`
- CI wiring (`.github/workflows/fuzz.yaml`, `Taskfile.yaml`) — tracked
  separately, alongside the other per-crate fuzz crates
