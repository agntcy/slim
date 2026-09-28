// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Verify path for the inter-node SLIM header MAC
//! (`HeaderMacSession::verify_slim_header` in
//! `crates/datapath/src/header_mac.rs`).
//!
//! Every message on a negotiated inter-node link carries a peer-supplied
//! `SlimHeader` and MAC tag; `verify_slim_header` is the boundary that decides
//! whether a live connection accepts it, and it is called once per message
//! for the whole lifetime of the link. This target keeps **one**
//! `HeaderMacSession` alive for the entire libFuzzer run rather than building
//! fresh state per input, mirroring that reuse: `verify_slim_header` clears
//! and refills a thread-local preimage buffer (`PREIMAGE_BUF`) on every call,
//! so anything that only misbehaves after many prior calls against the same
//! session -- unbounded growth in that buffer's retained capacity, or a panic
//! that only triggers for some *sequence* of header shapes and tag lengths --
//! needs a long, adversarial sequence of calls to surface, not just one.
//!
//! `header_mac.rs` itself has no numbered sequence/replay-window state of its
//! own (SLIM's message-id replay cache lives in a different crate, over
//! session-layer control messages); a single long-lived session driven
//! through a long sequence of crafted calls is this boundary's closest
//! analogue, so that is what this target stresses.
//!
//! Invariants:
//! - `verify_slim_header` never panics, for any header/tag/link_id.
//! - A tag whose length is not exactly 32 bytes is always rejected.
//! - A header we just signed always verifies, and flipping a single bit in
//!   its tag afterwards always breaks verification (forged MAC rejected).

#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use prost::Message as _;
use slim_datapath::api::SlimHeader;
use slim_datapath::header_mac::HeaderMacSession;

/// HMAC-SHA256 tag length used by `header_mac.rs` (kept private there).
const TAG_LEN: usize = 32;

/// One session, reused for the whole fuzzing run -- see module docs above.
static SESSION: LazyLock<HeaderMacSession> =
    LazyLock::new(|| HeaderMacSession::new(&[0x42; 32]).expect("32-byte key is always valid"));

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }

    // First two bytes steer how the rest is split; the fuzzer controls the
    // split points directly, so it can freely explore link_id length, tag
    // length (including, but not limited to, the real 32), and header shape.
    let link_id_len = (data[0] as usize) % 16;
    let tag_len_selector = (data[1] as usize) % 40; // covers 0..40, including 32
    let rest = &data[2..];
    if rest.len() < link_id_len {
        return;
    }
    let (link_id_bytes, rest) = rest.split_at(link_id_len);
    let Ok(link_id) = std::str::from_utf8(link_id_bytes) else {
        return;
    };
    let tag_len = tag_len_selector.min(rest.len());
    let (tag_bytes, header_bytes) = rest.split_at(tag_len);

    // Whatever `SlimHeader` the fuzzer can make of the remaining bytes; a
    // decode failure just falls back to a default header, since the
    // interesting surface here is the tag and the session state, not header
    // decoding (already covered by `message-decode` in the proto fuzz crate).
    let mut header = SlimHeader::decode(header_bytes).unwrap_or_default();
    header.header_mac = Some(tag_bytes.to_vec());

    // Invariant: a tag of the wrong length is always rejected, and verify
    // never panics regardless of what the tag or header contain.
    let result = SESSION.verify_slim_header(&header, link_id);
    if tag_bytes.len() != TAG_LEN {
        assert!(
            result.is_err(),
            "a {}-byte tag must never verify",
            tag_bytes.len()
        );
    }

    // Invariant: a freshly-signed tag must verify, and a single flipped bit
    // in it must never verify (forged MAC) -- run against the same
    // long-lived SESSION as above, so its internal state sees a long,
    // adversarial sequence of header shapes and tag lengths over the whole
    // fuzzing run rather than a single isolated call.
    if !link_id.is_empty() {
        let mut signed = header.clone();
        signed.header_mac = None;
        SESSION
            .sign_slim_header(&mut signed, link_id)
            .expect("non-empty link_id always signs");
        SESSION
            .verify_slim_header(&signed, link_id)
            .expect("a header we just signed must verify");

        let mut forged = signed.clone();
        let tag = forged.header_mac.as_mut().expect("sign always sets a tag");
        let last = tag.len() - 1;
        tag[last] ^= 0xFF;
        assert!(
            SESSION.verify_slim_header(&forged, link_id).is_err(),
            "a single flipped bit in a valid tag must never verify"
        );
    }
});
