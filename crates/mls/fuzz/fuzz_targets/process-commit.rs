// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Process arbitrary bytes as an MLS `Commit` message via `Mls::process_commit`.
//!
//! This mirrors what an already-joined group member does with every Commit
//! broadcast by another member (`crates/mls/src/mls.rs`): the bytes are fully
//! peer-controlled, handed straight to `MlsMessage::from_bytes` and then to
//! `Group::process_incoming_message`, before this peer has any way to
//! authenticate the sender. A panic here would take down a live session
//! rather than surface as a rejected commit.
//!
//! Invariant: no panic/unwrap/OOM on malformed input, and a *rejected* commit
//! must leave the group's epoch and group id exactly as they were — a bad
//! actor must not be able to desync or half-apply state onto a live group
//! merely by getting a message to `process_commit`.

#![no_main]

use std::sync::{Mutex, OnceLock};

use agntcy_slim_mls_fuzz::{init_joined_group, FuzzMls};
use libfuzzer_sys::fuzz_target;

// Built once and reused for every iteration — see crate-level docs in
// `src/lib.rs` for why that is sound here.
static GROUP: OnceLock<Mutex<FuzzMls>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let mut bob = GROUP
        .get_or_init(|| Mutex::new(init_joined_group()))
        .lock()
        .unwrap();

    let epoch_before = bob.get_epoch();
    let group_id_before = bob.get_group_id();

    match bob.process_commit(data) {
        Ok(()) => {
            // Accepted: the epoch may legitimately move, but the group
            // identity must never change underneath us.
            assert_eq!(
                bob.get_group_id(),
                group_id_before,
                "accepting a commit must not change the group id"
            );
        }
        Err(_) => {
            assert_eq!(
                bob.get_epoch(),
                epoch_before,
                "a rejected commit must not advance the epoch"
            );
            assert_eq!(
                bob.get_group_id(),
                group_id_before,
                "a rejected commit must not change the group id"
            );
        }
    }
});
