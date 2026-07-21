// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Post-quantum opt-in policy shared across TLS, link negotiation, and MLS.

use serde::{Deserialize, Serialize};

use crate::tls::errors::ConfigError as TlsConfigError;

/// Deployment-wide PQC policy. When enforced, all three layers (TLS hybrid KX,
/// link HMAC hybrid KEM, MLS ML-KEM ciphersuite) must use post-quantum algorithms.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnforcePqcPolicy(bool);

impl Default for EnforcePqcPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

impl EnforcePqcPolicy {
    pub const fn disabled() -> Self {
        Self(false)
    }

    pub const fn enforced() -> Self {
        Self(true)
    }

    pub fn is_enforced(self) -> bool {
        self.0
    }

    /// Hybrid PQ key exchange is TLS 1.3 only.
    pub fn validate_tls_version(self, tls_version: &str) -> Result<(), TlsConfigError> {
        if self.is_enforced() && tls_version != "tls1.3" {
            return Err(TlsConfigError::InvalidTlsVersion(
                "enforce_pqc requires tls_version \"tls1.3\"".into(),
            ));
        }
        Ok(())
    }
}

impl From<bool> for EnforcePqcPolicy {
    fn from(value: bool) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforced_requires_tls13() {
        assert!(
            EnforcePqcPolicy::enforced()
                .validate_tls_version("tls1.2")
                .is_err()
        );
        assert!(
            EnforcePqcPolicy::enforced()
                .validate_tls_version("tls1.3")
                .is_ok()
        );
    }

    #[test]
    fn disabled_allows_any_tls_version() {
        assert!(
            EnforcePqcPolicy::disabled()
                .validate_tls_version("tls1.2")
                .is_ok()
        );
    }
}
