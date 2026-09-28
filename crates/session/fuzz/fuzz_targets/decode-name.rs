// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Decode arbitrary bytes as a persisted [`ProtoName`].
//!
//! `decode_name` (`crates/session/src/persistence.rs`) is what turns the
//! `Vec<u8>` a session record stores for its `source`/`destination`/`control`
//! names, and the moderator's `mls_participants` map, back into a `ProtoName`
//! on restore. That KV entry is read back from disk (or a remote store)
//! rather than freshly produced by this process, so a corrupted or tampered
//! record is exactly what this decoder sees first — a panic here would take
//! down session restore instead of `PersistedSession::from_bytes`/restore
//! cleanly reporting `SessionError::PersistenceDecode`.
//!
//! Invariant: no panic and no allocation sized directly off an untrusted
//! length prefix (`decode_name` is a thin wrapper over prost's
//! `Message::decode`, which bounds every length-delimited field's read
//! against the buffer's actual remaining length before it copies — see
//! `prost::encoding::bytes::merge`/`merge_one_copy` — so a bogus huge length
//! prefix is rejected as `BufferUnderflow` rather than driving an allocation).
//! Also asserts `decode_name(encode) == x` for `Arbitrary`-generated valid
//! names, since a name that fails to round-trip is as much a bug as a panic.

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use prost::Message as _;
use slim_datapath::api::ProtoName;
use slim_session::fuzzing;

/// A valid `ProtoName` is fully determined by its three string components and
/// its (optional) id, so `Arbitrary` on this small struct is enough to build
/// one via the same public constructors real callers use.
#[derive(Debug, Arbitrary)]
struct NameInput {
    c0: String,
    c1: String,
    c2: String,
    id: u128,
}

fuzz_target!(|data: &[u8]| {
    // Fully untrusted bytes: most inputs are not a valid encoding at all, and
    // that must come back as `Err`, never a panic or a runaway allocation.
    let _ = fuzzing::decode_name(data);

    // Round-trip: build a valid name from the same fuzzer input, encode it
    // with the same prost `Message::encode_to_vec` a real caller would use
    // (`encode_name` in `persistence.rs` does nothing else), and check decode
    // recovers it unchanged.
    let mut u = Unstructured::new(data);
    if let Ok(input) = NameInput::arbitrary(&mut u) {
        let name = ProtoName::from_strings([input.c0, input.c1, input.c2]).with_id(input.id);
        let bytes = name.encode_to_vec();

        let decoded =
            fuzzing::decode_name(&bytes).expect("decoding a name we just encoded must not fail");
        assert_eq!(decoded, name, "decode_name(encode(x)) must equal x");
    }
});
