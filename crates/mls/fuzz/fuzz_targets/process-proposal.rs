// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Process arbitrary bytes as an MLS `Proposal` message via
//! `Mls::process_proposal`.
//!
//! This mirrors how a group member handles a Proposal broadcast by another
//! member — e.g. the credential-rotation proposals `create_rotation_proposal`
//! produces (`crates/mls/src/mls.rs`) — with the peer-controlled bytes handed
//! straight to `MlsMessage::from_bytes` and `Group::process_incoming_message`.
//! `process_proposal` additionally takes a `create_commit` flag chosen by the
//! caller from network context rather than from validated input, so the
//! first fuzzed byte selects it here and the rest is the proposal message.
//!
//! Invariant: no panic/unwrap/OOM on malformed input, and a *rejected*
//! proposal must not advance the epoch or leave a half-applied commit behind
//! — the group must come back exactly as it was. An *accepted* proposal
//! committed in the same call must advance the epoch by exactly one; staged
//! without committing, it must advance nothing and return no commit message.

#![no_main]

use std::sync::{Mutex, OnceLock};

use agntcy_slim_mls_fuzz::{init_joined_group, FuzzMls};
use libfuzzer_sys::fuzz_target;

// Built once and reused for every iteration — see crate-level docs in
// `src/lib.rs` for why that is sound here.
static GROUP: OnceLock<Mutex<FuzzMls>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    // First byte picks `create_commit`, so one corpus covers both call
    // shapes `process_proposal` supports; the remainder is the fuzzed
    // message.
    let Some((&flag, message)) = data.split_first() else {
        return;
    };
    let create_commit = flag & 1 == 1;

    let mut bob = GROUP
        .get_or_init(|| Mutex::new(init_joined_group()))
        .lock()
        .unwrap();

    let epoch_before = bob.get_epoch();
    let group_id_before = bob.get_group_id();

    match bob.process_proposal(message, create_commit) {
        Ok(commit) => {
            assert_eq!(
                bob.get_group_id(),
                group_id_before,
                "accepting a proposal must not change the group id"
            );
            if create_commit {
                assert!(
                    !commit.is_empty(),
                    "committing an accepted proposal must produce a commit message"
                );
                assert_eq!(
                    bob.get_epoch(),
                    epoch_before.map(|e| e + 1),
                    "committing a single accepted proposal must advance the epoch by exactly one"
                );
            } else {
                assert!(
                    commit.is_empty(),
                    "process_proposal(create_commit = false) must return no commit message"
                );
                assert_eq!(
                    bob.get_epoch(),
                    epoch_before,
                    "staging a proposal without committing must not advance the epoch"
                );
            }
        }
        Err(_) => {
            assert_eq!(
                bob.get_epoch(),
                epoch_before,
                "a rejected proposal must not advance the epoch"
            );
            assert_eq!(
                bob.get_group_id(),
                group_id_before,
                "a rejected proposal must not change the group id"
            );
        }
    }
});
