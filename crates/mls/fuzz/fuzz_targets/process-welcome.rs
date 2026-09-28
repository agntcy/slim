// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Process arbitrary bytes as an MLS `Welcome` message via `Mls::process_welcome`.
//!
//! This mirrors what a freshly-invited client does with the Welcome it gets
//! from whoever added it (`crates/mls/src/mls.rs`): the bytes are entirely
//! peer-supplied and are the *first* thing this client ever does with the
//! group, so there is no established state yet to check the sender against.
//! Unlike `process_commit`, acceptance here creates a brand-new group from
//! scratch rather than advancing an existing one.
//!
//! Invariant: no panic/unwrap/OOM on malformed input, and a *rejected*
//! welcome must not create or otherwise change the client's group — it must
//! come back exactly as groupless (or exactly as already-joined, if an
//! earlier iteration's welcome was legitimately accepted) as it went in.

#![no_main]

use std::sync::{Mutex, OnceLock};

use agntcy_slim_mls_fuzz::{init_client, FuzzMls};
use libfuzzer_sys::fuzz_target;

// Built once and reused for every iteration — see crate-level docs in
// `src/lib.rs` for why that is sound here.
static CLIENT: OnceLock<Mutex<FuzzMls>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let mut client = CLIENT
        .get_or_init(|| Mutex::new(init_client("charlie")))
        .lock()
        .unwrap();

    let epoch_before = client.get_epoch();
    let group_id_before = client.get_group_id();

    match client.process_welcome(data) {
        Ok(group_id) => {
            assert_eq!(
                Some(group_id),
                client.get_group_id(),
                "a successful join must report the group id it actually joined"
            );
            assert!(
                client.get_epoch().is_some(),
                "a successful join must leave a live group with a known epoch"
            );
        }
        Err(_) => {
            assert_eq!(
                client.get_epoch(),
                epoch_before,
                "a rejected welcome must not create or change a group"
            );
            assert_eq!(
                client.get_group_id(),
                group_id_before,
                "a rejected welcome must not create or change a group"
            );
        }
    }
});
