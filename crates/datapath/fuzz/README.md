# Fuzz targets

Each target mirrors a real boundary in `agntcy-slim-datapath` rather than
fuzzing for its own sake. See `crates/proto/fuzz/README.md` for the sibling
crate this one follows the conventions of.

- **`header-mac-verify`** — fuzzes `HeaderMacSession::verify_slim_header`
  (`crates/datapath/src/header_mac.rs`), the boundary that decides whether a
  peer-supplied `SlimHeader` and MAC tag are accepted on a negotiated
  inter-node link. It is called once per message for the whole lifetime of a
  link, so the target keeps **one** session alive for the entire fuzzing run
  instead of rebuilding state per input, driving it through a long,
  adversarial sequence of header shapes and tag lengths. This is the closest
  analogue available in `header_mac.rs` to a replay window: the module has no
  numbered sequence state of its own (SLIM's message-id replay cache lives in
  a different crate), but it does reuse a thread-local preimage buffer across
  every call, so a bug that only appears after many prior calls needs a
  sequence, not a single call, to surface. Asserts verify never panics, that a
  tag whose length isn't exactly 32 bytes is always rejected, and that a
  freshly signed tag always verifies while a single flipped bit in it never
  does (forged MAC rejected).

- **`link-ecdh-parse`** — fuzzes peer key material parsing in
  `crates/datapath/src/link_ecdh.rs`: an X25519 public key, an ML-KEM-768
  public key (encapsulation), and an ML-KEM-768 ciphertext (decapsulation),
  all accepted from an unauthenticated peer during link negotiation before
  any header MAC exists to validate the sender. `link_ecdh` has two
  independent backends — `backend_awslc` (native production, `aws_lc_rs`) and
  `backend_pure` (the wasm/browser production backend). **This target feeds
  the identical fuzzer-supplied bytes to both backends for all three checks
  and asserts they reach the same accept/reject decision** — a native↔browser
  link only works if both sides agree on what is valid, so a divergence
  between the two backends on the same input is a bug in its own right,
  independent of any panic. Reaching `backend_pure` on a native fuzz build
  needs the `fuzzing` feature on `agntcy-slim-datapath` (see
  `crates/datapath/Cargo.toml`), which compiles it as a real dependency
  instead of a test-only one and exposes both backends publicly under
  `link_ecdh::fuzzing_backend_awslc` / `link_ecdh::fuzzing_backend_pure` —
  the same "feature gate over a blanket pub" approach the proto fuzz crate's
  README recommends.

## Running

From this directory, with nightly on `PATH` (see "Why this is a separate
crate" below):

```bash
cargo fuzz build
cargo fuzz run header-mac-verify -- -max_total_time=60
cargo fuzz run link-ecdh-parse -- -max_total_time=60
```

`task fuzz` / `task fuzz:list` do not yet cover this crate — see
`crates/proto/fuzz/Taskfile.yaml` wiring, which is being extended to the new
fuzz crates separately.

## Why this is a separate crate

libFuzzer requires nightly. The `[workspace]` stanza at the bottom of
`Cargo.toml` detaches this crate from the repo root, so `cargo build
--workspace` never sees it and the nightly requirement stays in this
directory. The repo toolchain stays pinned to stable in
`rust-toolchain.toml`.

The tasks in the sibling `crates/proto/fuzz` crate select nightly by putting
its bin directory first on `PATH`, not with `cargo +nightly` or
`RUSTUP_TOOLCHAIN`. Both of those are rustup mechanisms and are silently
ignored when `cargo` on `PATH` is not a rustup proxy — a Homebrew `rust`
install, for example. `cargo-fuzz` also spawns a plain `cargo` of its own,
which has to resolve to nightly too.

## Adding a target

1. Add `fuzz_targets/<name>.rs` with a module doc comment naming the boundary
   it mirrors and the invariant it asserts.
2. Add a `[[bin]]` entry to `Cargo.toml`.
3. Add a bullet above, in the same form: what it parses, which real code path
   it stands in for, and why a panic (or, for `link-ecdh-parse`, a
   cross-backend divergence) there would matter.

Reaching a private module needs either widened visibility or a target inside
the crate; prefer a feature gate (see `fuzzing` on `agntcy-slim-datapath`)
over a blanket `pub` so the widened surface does not become de facto public
API.

## Not yet covered

- Seed corpora and a time-boxed CI job (tracked alongside the other new fuzz
  crates from #2076)
