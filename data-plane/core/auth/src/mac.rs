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
