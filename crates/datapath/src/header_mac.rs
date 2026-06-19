// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! HMAC-SHA256 integrity for SLIM headers on inter-node links.
//!
//! The preimage intentionally **excludes** `incoming_conn` (local connection index),
//! `header_mac`, and any session/payload fields so peers can set `incoming_conn`
//! after verify without breaking the MAC.
//!
//! Performance: [`HeaderMacSession`] holds only an [`hmac::Key`]. A thread-local
//! `Vec` is reused for the canonical preimage to avoid a per-connection mutex and
//! to cut allocator traffic on the hot path (see workspace performance skill notes).

use std::cell::RefCell;

use aws_lc_rs::hmac;
use thiserror::Error;

use crate::api::proto::dataplane::v1::SlimHeader;
use bytes::Bytes;

thread_local! {
    static PREIMAGE_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(512));
}

const DOMAIN_V1: &[u8] = b"SLIM-DP-HDR-v1\0";
const MIN_KEY_LEN: usize = 32;
const TAG_LEN: usize = 32;

#[derive(Debug, Error)]
pub enum HeaderMacError {
    #[error("header_mac key must be at least {MIN_KEY_LEN} bytes")]
    KeyTooShort,
    #[error("link_id must not be empty")]
    EmptyLinkId,
    #[error("missing SLIM header integrity tag")]
    MissingTag,
    #[error("invalid integrity tag length")]
    InvalidTagLength,
    #[error("SLIM header integrity verification failed")]
    VerificationFailed,
    #[error("inter-node link key agreement failed")]
    KeyAgreement,
    #[error("key generation failed")]
    KeyGenerationFailed(String),
}

/// Per-link HMAC state: only the key material. Preimage buffers are thread-local (see module docs).
#[derive(Clone)]
pub struct HeaderMacSession {
    key: hmac::Key,
}

impl std::fmt::Debug for HeaderMacSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeaderMacSession").finish_non_exhaustive()
    }
}

impl HeaderMacSession {
    /// Build session from raw secret bytes (≥32 bytes).
    pub fn new(secret: &[u8]) -> Result<Self, HeaderMacError> {
        if secret.len() < MIN_KEY_LEN {
            return Err(HeaderMacError::KeyTooShort);
        }
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, secret),
        })
    }

    /// Compute MAC and store it on `header` (clears any previous tag first).
    pub fn sign_slim_header(
        &self,
        header: &mut SlimHeader,
        link_id: &str,
    ) -> Result<(), HeaderMacError> {
        if link_id.is_empty() {
            return Err(HeaderMacError::EmptyLinkId);
        }
        header.header_mac = None;
        PREIMAGE_BUF.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();
            reserve_preimage_upper_bound(&mut buf, header, link_id);
            write_preimage(&mut buf, header, link_id.as_bytes());
            let tag = hmac::sign(&self.key, buf.as_slice());
            header.header_mac = Some(tag.as_ref().to_vec().into());
            Ok(())
        })
    }

    /// Verify MAC on `header` without mutating it.
    pub fn verify_slim_header(
        &self,
        header: &SlimHeader,
        link_id: &str,
    ) -> Result<(), HeaderMacError> {
        if link_id.is_empty() {
            return Err(HeaderMacError::EmptyLinkId);
        }
        let tag = header
            .header_mac
            .as_deref()
            .ok_or(HeaderMacError::MissingTag)?;
        if tag.len() != TAG_LEN {
            return Err(HeaderMacError::InvalidTagLength);
        }
        PREIMAGE_BUF.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();
            reserve_preimage_upper_bound(&mut buf, header, link_id);
            write_preimage(&mut buf, header, link_id.as_bytes());
            hmac::verify(&self.key, buf.as_slice(), tag)
                .map_err(|_| HeaderMacError::VerificationFailed)
        })
    }
}

/// Ensure `buf` can hold the worst-case preimage without reallocation during `write_preimage`.
#[inline]
fn reserve_preimage_upper_bound(buf: &mut Vec<u8>, hdr: &SlimHeader, link_id: &str) {
    let need = preimage_upper_bound(hdr, link_id);
    let cap = buf.capacity();
    if need > cap {
        buf.reserve(need - cap);
    }
}

#[inline]
fn preimage_upper_bound(header: &SlimHeader, link_id: &str) -> usize {
    /// Byte size of the link_id length prefix (u32).
    const LINK_ID_LEN_PREFIX: usize = 4;

    /// Byte size of `fanout`, serialized by [`to_le_bytes`]
    /// * 4 bytes for `u32`
    const FANOUT_SIZE: usize = 4;

    /// The total byte size of a `recv_from` field.
    ///
    /// This is calculated as 9 bytes:
    /// * 1 byte for the presence tag (boolean `0` or `1`).
    /// * 8 bytes for the `u64` value (via [`push_u64_opt`]).
    const RECV_FROM_SIZE: usize = 9;

    /// The total byte size of a `forward_to` field.
    ///
    /// This is calculated as 9 bytes:
    /// * 1 byte for the presence tag (boolean `0` or `1`).
    /// * 8 bytes for the `u64` value (via [`push_u64_opt`]).
    const FORWARD_TO_SIZE: usize = 9;

    /// Byte size of error: Option<bool> encoded by [`push_bool_opt`]
    const ERROR_SIZE: usize = 2;

    /// Byte size of TTL: u32 serialized by [`to_le_bytes`]
    const TTL_SIZE: usize = 4;

    DOMAIN_V1.len()
        + LINK_ID_LEN_PREFIX
        + link_id.len()
        + FANOUT_SIZE
        + RECV_FROM_SIZE
        + FORWARD_TO_SIZE
        + ERROR_SIZE
        + TTL_SIZE
        + encoded_name_upper_bound(&header.source, &header.source_str)
        + encoded_name_upper_bound(&header.destination, &header.destination_str)
}

#[inline]
fn encoded_name_upper_bound(encoded: &Bytes, str_name: &Bytes) -> usize {
    // 1 byte: encoded presence + 40 bytes (when present)
    // 1 byte: str_name presence + 4 bytes len prefix + N str bytes (when present)
    1 + if encoded.is_empty() { 0 } else { 40 }
        + 1 + if str_name.is_empty() { 0 } else { 4 + str_name.len() }
}

#[inline]
fn push_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buf.extend_from_slice(data);
}

#[inline]
fn push_u64_opt(buf: &mut Vec<u8>, v: Option<u64>) {
    match v {
        None => buf.push(0),
        Some(x) => {
            buf.push(1);
            buf.extend_from_slice(&x.to_le_bytes());
        }
    }
}

#[inline]
fn push_bool_opt(buf: &mut Vec<u8>, v: Option<bool>) {
    match v {
        None => buf.push(0),
        Some(b) => {
            buf.push(1);
            buf.push(u8::from(b));
        }
    }
}

fn push_encoded_name(buf: &mut Vec<u8>, encoded: &Bytes, str_name: &Bytes) {
    // encoded: presence byte + raw 40-byte flat encoding
    if encoded.is_empty() {
        buf.push(0);
    } else {
        buf.push(1);
        buf.extend_from_slice(encoded);
    }
    // str_name: presence byte + u32 LE total length + raw packed bytes
    if str_name.is_empty() {
        buf.push(0);
    } else {
        buf.push(1);
        buf.extend_from_slice(&(str_name.len() as u32).to_le_bytes());
        buf.extend_from_slice(str_name);
    }
}

/// Canonical preimage: domain || len(link_id) || link_id || routing header fields (no incoming_conn, no tag).
fn write_preimage(buf: &mut Vec<u8>, hdr: &SlimHeader, link_id_bytes: &[u8]) {
    buf.extend_from_slice(DOMAIN_V1);
    buf.extend_from_slice(&(link_id_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(link_id_bytes);
    push_encoded_name(buf, &hdr.source, &hdr.source_str);
    push_encoded_name(buf, &hdr.destination, &hdr.destination_str);
    push_bytes(buf, &hdr.identity);
    buf.extend_from_slice(&hdr.fanout.to_le_bytes());
    push_u64_opt(buf, hdr.recv_from);
    push_u64_opt(buf, hdr.forward_to);
    push_bool_opt(buf, hdr.error);
    buf.extend_from_slice(&hdr.ttl.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProtoName;
    use crate::messages::utils::DEFAULT_TTL;
    use uuid::Uuid;

    fn test_key() -> Vec<u8> {
        b"01234567890123456789012345678901".to_vec()
    }

    fn sample_header() -> SlimHeader {
        use crate::messages::utils::SlimHeaderFlags;
        SlimHeader::new(
            ProtoName::from_strings(["a", "b", "c"]).with_id(4),
            ProtoName::from_strings(["x", "y", "z"]).with_id(8),
            "id1",
            Some(SlimHeaderFlags {
                fanout: 2,
                recv_from: Some(9),
                forward_to: Some(10),
                incoming_conn: Some(999),
                error: Some(false),
                ttl: DEFAULT_TTL,
            }),
        )
    }

    #[test]
    fn sign_verify_round_trip() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();
        mac.sign_slim_header(&mut hdr, &lid).unwrap();
        mac.verify_slim_header(&hdr, &lid).unwrap();
    }

    #[test]
    fn incoming_conn_does_not_affect_mac() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut h1 = sample_header();
        mac.sign_slim_header(&mut h1, &lid).unwrap();
        let mut h2 = h1.clone();
        h2.incoming_conn = Some(12345);
        mac.verify_slim_header(&h2, &lid).unwrap();
    }

    #[test]
    fn tampered_fanout_fail() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();
        mac.sign_slim_header(&mut hdr, &lid).unwrap();
        hdr.fanout = 3;
        assert!(mac.verify_slim_header(&hdr, &lid).is_err());
    }

    #[test]
    fn tampered_mac_fail() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();
        mac.sign_slim_header(&mut hdr, &lid).unwrap();
        // HMAC tag is tampered
        let mut v = hdr.header_mac.take().unwrap().to_vec();
        v[0] ^= 1;
        hdr.header_mac = Some(v.into());
        let err = mac.verify_slim_header(&hdr, &lid).unwrap_err();
        assert!(matches!(err, HeaderMacError::VerificationFailed));
    }

    #[test]
    fn cross_link_replay_fail() {
        let lid1 = Uuid::new_v4().to_string();
        let lid2 = Uuid::new_v4().to_string();
        assert_ne!(lid1, lid2);

        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();

        // Sign for link 1
        mac.sign_slim_header(&mut hdr, &lid1).unwrap();

        // Verification for link 2 must fail even if the key is the same,
        // because the link_id is part of the MAC preimage.
        let err = mac.verify_slim_header(&hdr, &lid2).unwrap_err();
        assert!(matches!(err, HeaderMacError::VerificationFailed));
    }

    #[test]
    fn name_id_none_sign_verify() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();
        // Clear the id from source (reset to NULL_COMPONENT) via ProtoName helper
        let mut src = hdr.get_source();
        src.reset_id();
        hdr.set_source(src);
        mac.sign_slim_header(&mut hdr, &lid).unwrap();
        mac.verify_slim_header(&hdr, &lid).unwrap();
    }

    #[test]
    fn tampered_name_id_fail() {
        let lid = Uuid::new_v4().to_string();
        let mac = HeaderMacSession::new(&test_key()).unwrap();
        let mut hdr = sample_header();
        mac.sign_slim_header(&mut hdr, &lid).unwrap();
        // Tamper with source id — equivalent to old NameId { id_0: 99, id_1: 99 }
        let mut src = hdr.get_source();
        src.set_id((99u128 << 64) | 99u128);
        hdr.set_source(src);
        assert!(mac.verify_slim_header(&hdr, &lid).is_err());
    }
}
