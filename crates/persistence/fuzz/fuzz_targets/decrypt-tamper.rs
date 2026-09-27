// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Decrypt arbitrary and deliberately-tampered blobs with `ValueCipher`.
//!
//! `ValueCipher::decrypt` (`crates/persistence/src/cipher.rs`) is the last
//! line of defense between the plain (unencrypted) at-rest SQLite database
//! and the MLS group snapshots / session records stored in it: every value
//! read back out of the store passes through it before the rest of the code
//! ever sees plaintext. A store on disk can be truncated by a crash mid-write,
//! edited, or swapped out from under the process, so this is exactly the
//! boundary where corrupted or actively-tampered bytes first arrive.
//!
//! Invariant, in two parts:
//!
//! - No panic on arbitrary bytes, including ones shorter than the 12-byte
//!   nonce prefix `decrypt` splits off before it ever touches AEAD.
//! - AES-256-GCM authentication must not be bypassable: a modified blob must
//!   never decrypt successfully. Fully random bytes trivially satisfy this
//!   (almost nothing coincidentally passes AEAD authentication), so that case
//!   alone doesn't exercise the property that matters. Instead, this target
//!   encrypts a known plaintext with a real key first, then uses the fuzzer
//!   input to pick a mutation — truncate, extend, flip one bit anywhere, or
//!   flip bits only within the nonce — and applies it to that real ciphertext.
//!   If `decrypt` ever returns `Ok` for a blob that isn't byte-identical to
//!   the original, that's a truncation/nonce-manipulation bypass of AEAD
//!   authentication.

#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use slim_persistence::fuzzing;

/// AES-256-GCM nonce length in `ValueCipher`'s `nonce || ciphertext||tag`
/// layout (`crates/persistence/src/cipher.rs`, `NONCE_LEN`).
const NONCE_LEN: usize = 12;

const IDENTITY: &str = "fuzz-identity";
const PLAINTEXT: &[u8] = b"fuzz secret payload";

fuzz_target!(|data: &[u8]| {
    // Case 1: fully untrusted bytes, most of which aren't a valid blob at
    // all (too short, garbage nonce, garbage tag). Must not panic.
    let _ = fuzzing::decrypt(IDENTITY, data);

    if data.is_empty() {
        return;
    }

    // Case 2: a real ciphertext, mutated. This is the case that actually
    // exercises AEAD authentication rather than trivially failing on noise.
    let ct = fuzzing::encrypt(IDENTITY, PLAINTEXT).expect("encrypt must succeed");
    let mut u = Unstructured::new(data);
    let mutated = mutate(&ct, &mut u);

    match fuzzing::decrypt(IDENTITY, &mutated) {
        Ok(pt) => {
            // The only way `Ok` is legitimate here is if the "mutation"
            // happened to reproduce the exact original blob (e.g. an empty
            // extension and a truncation to the full length) — anything
            // that actually changed the bytes and still decrypted is an
            // authentication bypass.
            assert_eq!(
                mutated, ct,
                "decrypt accepted a modified blob without rejecting it \
                 (AEAD authentication bypass)"
            );
            assert_eq!(
                pt, PLAINTEXT,
                "decrypt returned the wrong plaintext for an unmodified blob"
            );
        }
        Err(_) => {
            // Expected: truncated, extended, or bit-flipped ciphertexts must
            // be rejected.
        }
    }
});

/// Apply one of four tamper strategies to a real `nonce || ciphertext||tag`
/// blob, chosen and parameterized by the fuzzer input.
fn mutate(original: &[u8], u: &mut Unstructured) -> Vec<u8> {
    let mut out = original.to_vec();

    match u.int_in_range(0u8..=3).unwrap_or(0) {
        0 => {
            // Truncate to an arbitrary length in [0, len] -- covers cutting
            // into the nonce, the ciphertext, and the trailing tag.
            let cut = u.int_in_range(0..=out.len()).unwrap_or(0);
            out.truncate(cut);
        }
        1 => {
            // Extend with arbitrary trailing bytes.
            let extra: Vec<u8> = u.arbitrary().unwrap_or_default();
            out.extend_from_slice(&extra);
        }
        2 => {
            // Flip one bit anywhere in the blob (nonce, ciphertext, or tag).
            if !out.is_empty() {
                let idx = u.int_in_range(0..=out.len() - 1).unwrap_or(0);
                let mask = u.arbitrary::<u8>().unwrap_or(1).max(1);
                out[idx] ^= mask;
            }
        }
        _ => {
            // Flip bits only within the nonce prefix, leaving
            // ciphertext/tag untouched -- the "nonce manipulation" case
            // named explicitly in the issue.
            let nonce_len = NONCE_LEN.min(out.len());
            for byte in out.iter_mut().take(nonce_len) {
                if u.arbitrary::<bool>().unwrap_or(false) {
                    *byte ^= 0xFF;
                }
            }
        }
    }

    out
}
