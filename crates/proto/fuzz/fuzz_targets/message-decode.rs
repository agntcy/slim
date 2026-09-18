// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Decode arbitrary bytes as a dataplane `Message`.
//!
//! This is where raw network bytes first become a typed message: the
//! WebSocket read loop in `crates/datapath/src/websocket/stream.rs` hands
//! every binary frame's payload straight to `Message::decode`. The payload is
//! entirely peer-controlled and is not authenticated at that point, so the
//! decode has to reject malformed input rather than panic the read loop out
//! from under a live connection.
//!
//! Invariant: no input panics, and anything that decodes must survive a
//! re-encode and decode unchanged.

#![no_main]

use agntcy_slim_proto::dataplane::proto::v1::Message;
use libfuzzer_sys::fuzz_target;
use prost::Message as _;

fuzz_target!(|data: &[u8]| {
    // A decode error is the correct outcome for almost every input; only the
    // absence of a panic matters here.
    if let Ok(msg) = Message::decode(data) {
        // prost round-trips are not byte-identical in general (field ordering
        // and default elision), so assert on the decoded value rather than on
        // the bytes: a message we accepted must re-encode and come back equal.
        let mut buf = Vec::with_capacity(msg.encoded_len());
        msg.encode(&mut buf)
            .expect("encoding a message we just decoded must not fail");

        let again =
            Message::decode(&buf[..]).expect("re-decoding our own encoding must not fail");

        assert_eq!(msg, again, "decode -> encode -> decode must be stable");
    }
});
