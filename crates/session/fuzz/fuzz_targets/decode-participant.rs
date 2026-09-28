// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Decode arbitrary bytes as a persisted [`Participant`].
//!
//! `decode_participant` (`crates/session/src/persistence.rs`) turns each
//! `Vec<u8>` entry of a session record's `group_list` back into a
//! `Participant` on restore. Like `decode_name`, this reads back a KV entry
//! from disk rather than a value this process just produced, so a corrupted
//! or tampered record is exactly what this decoder sees first — a panic here
//! would take down session restore instead of cleanly reporting
//! `SessionError::PersistenceDecode`.
//!
//! Invariant: no panic and no allocation sized directly off an untrusted
//! length prefix (`decode_participant` is a thin wrapper over prost's
//! `Message::decode`, which bounds every length-delimited field's read
//! against the buffer's actual remaining length before it copies — see
//! `prost::encoding::bytes::merge`/`merge_one_copy` — so a bogus huge length
//! prefix is rejected as `BufferUnderflow` rather than driving an
//! allocation). Also asserts `decode_participant(encode(x)) == x` for
//! `Arbitrary`-generated valid participants.

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use prost::Message as _;
use slim_datapath::api::{Participant, ParticipantSettings, ProtoName};
use slim_session::fuzzing;

#[derive(Debug, Arbitrary)]
struct ParticipantInput {
    c0: String,
    c1: String,
    c2: String,
    id: u128,
    sends_data: bool,
    receives_data: bool,
}

fuzz_target!(|data: &[u8]| {
    // Fully untrusted bytes: most inputs are not a valid encoding at all, and
    // that must come back as `Err`, never a panic or a runaway allocation.
    let _ = fuzzing::decode_participant(data);

    // Round-trip: build a valid participant from the same fuzzer input,
    // encode it with the same prost `Message::encode_to_vec` a real caller
    // would use (`encode_participant` in `persistence.rs` does nothing
    // else), and check decode recovers it unchanged.
    let mut u = Unstructured::new(data);
    if let Ok(input) = ParticipantInput::arbitrary(&mut u) {
        let name = ProtoName::from_strings([input.c0, input.c1, input.c2]).with_id(input.id);
        let settings = ParticipantSettings {
            sends_data: input.sends_data,
            receives_data: input.receives_data,
        };
        let participant = Participant::new(name, settings);
        let bytes = participant.encode_to_vec();

        let decoded = fuzzing::decode_participant(&bytes)
            .expect("decoding a participant we just encoded must not fail");
        assert_eq!(
            decoded, participant,
            "decode_participant(encode(x)) must equal x"
        );
    }
});
