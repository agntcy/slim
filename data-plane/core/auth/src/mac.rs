// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! HMAC-SHA256 abstraction used by [`crate::shared_secret::SharedSecret`].
//!
//! Two interchangeable backends are provided so the same token format works on
//! every target:
//! * Native targets use `aws-lc-rs` for FIPS-friendly, constant-time
//!   primitives, and the derived key is cached for the lifetime of the
//!   provider.
//! * `wasm32` targets use the pure-Rust `hmac` + `sha2` crates (browser
//!   WebCrypto is async-only and cannot back this synchronous API).
//!
//! Both backends produce identical tags and perform constant-time
//! verification.

/// HMAC-SHA256 tag length in bytes.
pub const HMAC_TAG_LEN: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use aws_lc_rs::hmac;

    /// Precomputed HMAC-SHA256 key (native backend).
    #[derive(Clone)]
    pub struct HmacKey(hmac::Key);

    impl HmacKey {
        pub fn new(secret: &[u8]) -> Self {
            HmacKey(hmac::Key::new(hmac::HMAC_SHA256, secret))
        }

        pub fn sign(&self, message: &[u8]) -> [u8; super::HMAC_TAG_LEN] {
            let tag = hmac::sign(&self.0, message);
            let mut out = [0u8; super::HMAC_TAG_LEN];
            out.copy_from_slice(tag.as_ref());
            out
        }

        /// Constant-time tag verification.
        pub fn verify(&self, message: &[u8], expected: &[u8]) -> bool {
            hmac::verify(&self.0, message, expected).is_ok()
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    /// HMAC-SHA256 key material (wasm backend). The RustCrypto `Hmac` state is
    /// consumed on finalize, so we keep the raw secret and build a fresh MAC
    /// per operation.
    #[derive(Clone)]
    pub struct HmacKey(Vec<u8>);

    impl HmacKey {
        pub fn new(secret: &[u8]) -> Self {
            HmacKey(secret.to_vec())
        }

        pub fn sign(&self, message: &[u8]) -> [u8; super::HMAC_TAG_LEN] {
            // `new_from_slice` accepts keys of any length for HMAC.
            let mut mac = HmacSha256::new_from_slice(&self.0).expect("HMAC accepts any key length");
            mac.update(message);
            let mut out = [0u8; super::HMAC_TAG_LEN];
            out.copy_from_slice(&mac.finalize().into_bytes());
            out
        }

        /// Constant-time tag verification.
        pub fn verify(&self, message: &[u8], expected: &[u8]) -> bool {
            let mut mac = HmacSha256::new_from_slice(&self.0).expect("HMAC accepts any key length");
            mac.update(message);
            mac.verify_slice(expected).is_ok()
        }
    }
}

pub use imp::HmacKey;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_is_deterministic_and_verifies() {
        let key = HmacKey::new(b"super-secret-key-material-0123456");
        let msg = b"hello world";
        let tag1 = key.sign(msg);
        let tag2 = key.sign(msg);
        assert_eq!(tag1, tag2);
        assert_eq!(tag1.len(), HMAC_TAG_LEN);
        assert!(key.verify(msg, &tag1));
    }

    #[test]
    fn verify_rejects_wrong_inputs() {
        let key = HmacKey::new(b"super-secret-key-material-0123456");
        let msg = b"hello world";
        let tag = key.sign(msg);

        // Wrong message.
        assert!(!key.verify(b"hello worlt", &tag));

        // Tampered tag.
        let mut tampered = tag;
        tampered[0] ^= 0xFF;
        assert!(!key.verify(msg, &tampered));

        // Wrong key.
        let other = HmacKey::new(b"different-key-material-9876543210");
        assert!(!other.verify(msg, &tag));

        // Wrong-length tag.
        assert!(!key.verify(msg, &tag[..16]));
    }

    #[test]
    fn different_keys_produce_different_tags() {
        let a = HmacKey::new(b"key-aaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let b = HmacKey::new(b"key-bbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_ne!(a.sign(b"msg"), b.sign(b"msg"));
    }
}
