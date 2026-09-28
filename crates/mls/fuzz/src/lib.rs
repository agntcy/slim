// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Shared group-setup scaffolding for the MLS fuzz targets.
//!
//! `process_commit`, `process_welcome` and `process_proposal` are stateful:
//! each operates on a live MLS group rather than parsing a value in
//! isolation, so a useful harness needs one built and ready before the
//! fuzzer's bytes ever reach the code under test. Building a fresh group on
//! every iteration (as the stateless parsers in the `agntcy-slim-proto` fuzz
//! crate can afford to do) would spend nearly the whole time budget on
//! `mls-rs` group setup rather than on the code being fuzzed.
//!
//! Instead, each target builds its group **once** behind a
//! `std::sync::OnceLock<std::sync::Mutex<_>>` and reuses it for every
//! iteration. This is sound precisely because the invariant under test is
//! that a *rejected* `process_*` call leaves the group unchanged: iterating
//! against one long-lived, shared group is exactly the scenario that
//! invariant describes. An *accepted* call is expected to legitimately
//! advance the group, and is left free to do so across iterations.
//!
//! This harness only targets native. `mls-rs` (via `maybe-async`) is
//! synchronous on native and asynchronous on `wasm32`; fuzzing the async
//! wasm32 shape would need a browser or a wasm executor rather than plain
//! `cargo fuzz`, so it is out of scope here.

use slim_auth::shared_secret::SharedSecret;
use slim_mls::mls::Mls;

/// Shared secret used for every identity the harness builds. It only needs to
/// satisfy `SharedSecret`'s minimum-length requirement — nothing under
/// fuzzing depends on it staying secret.
const FUZZ_SHARED_SECRET: &str = "mls-fuzz-harness-shared-secret-do-not-reuse-elsewhere-000000";

/// The concrete `Mls` instantiation every fuzz target drives.
pub type FuzzMls = Mls<SharedSecret, SharedSecret>;

fn identity(name: &str) -> SharedSecret {
    SharedSecret::new(name, FUZZ_SHARED_SECRET).expect("fuzz harness: build shared-secret identity")
}

/// Run a future to completion on a fresh current-thread runtime.
///
/// `Mls::initialize` is always async, even on native, because it goes through
/// `TokenProvider`, which is `async_trait`-based on every target. Everything
/// else the harnesses call (`create_group`, `generate_key_package`,
/// `add_member`, `process_welcome`, `process_commit`, `process_proposal`,
/// ...) is synchronous on native via `maybe-async`'s `is_sync` feature and
/// needs no runtime at all.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("fuzz harness: build tokio runtime")
        .block_on(future)
}

/// Build and initialize a bare client with no group yet — the exact state a
/// member sits in right before it receives its first `Welcome`.
pub fn init_client(name: &str) -> FuzzMls {
    let mut mls = FuzzMls::new(identity(name), identity(name));
    block_on(mls.initialize()).expect("fuzz harness: Mls::initialize");
    mls
}

/// Build a two-member group — `alice` as moderator, `bob` already joined —
/// and return `bob`, the instance `process_commit`/`process_proposal` fuzz.
/// `alice` is dropped: her only job was producing a legitimate add-member
/// commit/welcome pair to get `bob` into a live group.
pub fn init_joined_group() -> FuzzMls {
    let mut alice = init_client("alice");
    let mut bob = init_client("bob");

    alice
        .create_group()
        .expect("fuzz harness: Mls::create_group");
    let bob_key_package = bob
        .generate_key_package()
        .expect("fuzz harness: Mls::generate_key_package");
    let add = alice
        .add_member(&bob_key_package)
        .expect("fuzz harness: Mls::add_member");
    // Not fuzzed: this is the harness's own well-formed setup welcome, not
    // attacker input.
    bob.process_welcome(&add.welcome_message)
        .expect("fuzz harness: Mls::process_welcome (setup)");

    bob
}
