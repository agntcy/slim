// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Utility for assembling a `rustls::RootCertStore`
//! from multiple optional sources (system roots, PEM strings, files, and
//! SPIFFE / SPIRE bundles).
//!
//! This isolates certificate aggregation logic from the broader TLS
//! configuration code, simplifying reasoning and testability.
//!
//! # Usage
//! ```ignore
//! use crate::tls::root_store_builder::RootStoreBuilder;
//! use crate::tls::common::TlsSource;
//!
//! async fn build() -> Result<rustls::RootCertStore, crate::tls::common::ConfigError> {
//!     let tls_source = TlsSource::File {
//!         ca: Some("/etc/ssl/certs/ca-bundle.crt".into()),
//!         cert: None,
//!         key: None,
//!     };
//!
//!     let store = RootStoreBuilder::new()
//!         .with_system_roots()
//!         .add_source(&tls_source).await?
//!         .finish()?;
//!     Ok(store)
//! }
//! ```
//!
//! # Design Notes
//! - System root loading is deferred until `finish()`, mirroring builder semantics.
//! - Each `add_*` method consumes and returns `Self` for fluent chaining.
//! - SPIFFE support is optional; if the socket path is not set, no bundle is added.
//! - Errors are converted into the existing `ConfigError` enum for consistency.

use std::path::Path;

use crate::auth::spiffe;
use crate::tls::common::{CaSource, ConfigError};
use rustls::RootCertStore;
use rustls_pki_types::{CertificateDer, pem::PemObject};

/// Builder for constructing a RootCertStore from multiple certificate sources.
pub struct RootStoreBuilder {
    store: RootCertStore,
    include_system: bool,
}

impl Default for RootStoreBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RootStoreBuilder {
    /// Create a new (empty) builder.
    pub fn new() -> Self {
        Self {
            store: RootCertStore::empty(),
            include_system: false,
        }
    }

    /// Enable inclusion of platform/system root certificates when `finish()` is called.
    pub fn with_system_roots(mut self) -> Self {
        self.include_system = true;
        self
    }

    /// Add CA certificates from a file containing one or more PEM-encoded certificates.
    pub fn add_file(mut self, path: &str) -> Result<Self, ConfigError> {
        let cert_path = Path::new(path);
        let iter = CertificateDer::pem_file_iter(cert_path).map_err(ConfigError::InvalidPem)?;
        for item in iter {
            let cert = item.map_err(ConfigError::InvalidPem)?;
            self.store.add(cert).map_err(ConfigError::RootStore)?;
        }
        Ok(self)
    }

    /// Add CA certificates from a PEM string containing one or more concatenated certs.
    pub fn add_pem(mut self, data: &str) -> Result<Self, ConfigError> {
        for item in CertificateDer::pem_slice_iter(data.as_bytes()) {
            let cert = item.map_err(ConfigError::InvalidPem)?;
            self.store.add(cert).map_err(ConfigError::RootStore)?;
        }
        Ok(self)
    }

    /// Add SPIFFE / SPIRE bundle authorities (only if a socket path is configured).
    pub async fn add_spiffe(mut self, cfg: &spiffe::SpiffeConfig) -> Result<Self, ConfigError> {
        Self::add_spiffe_bundles(&mut self.store, cfg).await?;
        Ok(self)
    }

    /// Convenience method: add from a TlsSource (File / Pem / Spire / None).
    pub async fn add_source(mut self, source: &CaSource) -> Result<Self, ConfigError> {
        match source {
            CaSource::File { path, .. } => {
                self = self.add_file(path)?;
            }
            CaSource::Pem { data, .. } => {
                self = self.add_pem(data)?;
            }
            CaSource::Spire { config, .. } => {
                self = self.add_spiffe(config).await?;
            }
            CaSource::None => { /* no-op */ }
        }

        Ok(self)
    }

    async fn add_spiffe_bundles(
        root_store: &mut RootCertStore,
        spiffe_cfg: &spiffe::SpiffeConfig,
    ) -> Result<(), ConfigError> {
        let spire_identity_manager = spiffe_cfg.create_provider().await.map_err(|e| {
            ConfigError::InvalidFile(format!("failed to create SPIFFE provider: {}", e))
        })?;

        if !spiffe_cfg.trust_domains.is_empty() {
            for domain in &spiffe_cfg.trust_domains {
                let bundle = spire_identity_manager
                    .get_x509_bundle_for_trust_domain(domain)
                    .map_err(|e| {
                        ConfigError::Spire(format!(
                            "failed to get X.509 bundle for trust domain {}: {}",
                            domain, e
                        ))
                    })?;

                for cert in bundle.authorities() {
                    let der_cert = CertificateDer::from(cert.as_ref().to_vec());
                    root_store.add(der_cert).map_err(ConfigError::RootStore)?;
                }
            }
        }

        let default_bundle = spire_identity_manager.get_x509_bundle().map_err(|e| {
            ConfigError::Spire(format!("failed to get default X.509 bundle: {}", e))
        })?;

        for cert in default_bundle.authorities() {
            let der_cert = CertificateDer::from(cert.as_ref().to_vec());
            root_store.add(der_cert).map_err(ConfigError::RootStore)?;
        }

        Ok(())
    }

    /// Internal: load system roots if requested.
    fn load_system_roots(&mut self) -> Result<(), ConfigError> {
        if self.include_system {
            let native_certs = rustls_native_certs::load_native_certs();
            for cert in native_certs.certs {
                self.store.add(cert).map_err(ConfigError::RootStore)?;
            }
        }
        Ok(())
    }

    /// Finalize and return the constructed RootCertStore.
    pub fn finish(mut self) -> Result<RootCertStore, ConfigError> {
        self.load_system_roots()?;
        Ok(self.store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;
    use std::fs::{self, File};
    use std::io::Write;

    // A single valid (truncated chain-safe) test certificate (Let’s Encrypt R3 root-derived).
    // This certificate should parse correctly; if parsing fails in future rustls versions,
    // tests asserting >0 length can be relaxed accordingly.
    // spellchecker:off
    const TEST_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIFazCCA1OgAwIBAgISA1/3cxguPmyAnbcW2CYwiV2lMA0GCSqGSIb3DQEBCwUA
MEoxCzAJBgNVBAYTAlVTMRYwFAYDVQQKEw1MZXQncyBFbmNyeXB0MSMwIQYDVQQD
ExpMZXQncyBFbmNyeXB0IEF1dGhvcml0eSBYMzAeFw0yMzA4MjkxOTExMDBaFw0y
MzExMjcxOTExMDBaMBYxFDASBgNVBAMTC2V4YW1wbGUuY29tMIIBIjANBgkqhkiG
9w0BAQEFAAOCAQ8AMIIBCgKCAQEA2HGhf0cPZXyMw4E8DqRLVQzAg/PvU+eR2qQ5
RPP9Z0sQ9Sg0wWEJ2MCcHw2r9Z8N+MsaQwE1ssBt0qjVt5Q50E9Q1Py27KbR3wQe
J5y7H3JzO8wG9+TZQ5A7gwr3n0kP5rX1S9x9z9X0oNk9p+4Y2xX9zGkQ50m+g3xn
1QIDAQABo4ICFjCCAhIwDgYDVR0PAQH/BAQDAgWgMB0GA1UdJQQWMBQGCCsGAQUF
BwMBBggrBgEFBQcDAjAMBgNVHRMBAf8EAjAAMB0GA1UdDgQWBBRNRy4pz50jdK5C
l4Jr2n7+G2kZZjAfBgNVHSMEGDAWgBQULrMXt1hWy65QCUDmH6+dixTCxjBVBggr
BgEFBQcBAQRJMEcwIQYIKwYBBQUHMAGGFWh0dHA6Ly9yMy5vLmxlbmNyLm9yZzAi
BggrBgEFBQcwAoYWaHR0cDovL3IzLmkubGVuY3Iub3JnLzAnBgNVHREEIDAeggxl
eGFtcGxlLmNvbYILd3d3LmV4LmNvbYIFZXguY29tMC4GA1UdHwQnMCUwI6AloCOG
IWh0dHA6Ly9jcmwubGV0c2VuY3J5cHQub3JnL3gzLmNybDAwBgNVHSAEKTAnMAgG
BmeBDAECATAIBgZngQwBAgIwCwYJYIZIAYb9bAIBMA0GCSqGSIb3DQEBCwUAA4IB
AQAobHxPPHkSsZQeU6Dkv1X48xlWDi8Jzb9Uf8jbx6Ui0Ic9I6kfiwJtaTGB2KUh
3N5l1DgweO2YpZJYG1ChcxrFAuo0xO+ogzAm8h1Hn0pV3hokW2N1DbStO2Qe6hw2
I6hwIDAQAB
-----END CERTIFICATE-----"#;
    // spellchecker:on

    fn write_temp(contents: &str) -> String {
        let mut rng = rand::rng();
        let suffix: u32 = rng.random();
        let path = format!("/tmp/test-cert-{}.pem", suffix);
        let mut f = File::create(&path).expect("create temp cert file");
        f.write_all(contents.as_bytes()).expect("write cert");
        path
    }

    #[test]
    fn test_empty_builder_no_system() {
        let store = RootStoreBuilder::new().finish().expect("finish");
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_add_pem_single() {
        let store = RootStoreBuilder::new()
            .add_pem(TEST_CERT_PEM)
            .expect("add pem")
            .finish()
            .expect("finish");
        assert!(!store.is_empty());
    }

    #[test]
    fn test_add_file_single() {
        let path = write_temp(TEST_CERT_PEM);
        let store = RootStoreBuilder::new()
            .add_file(&path)
            .expect("add file")
            .finish()
            .expect("finish");
        assert!(!store.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_add_pem_then_file_accumulates() {
        let path = write_temp(TEST_CERT_PEM);
        let store = RootStoreBuilder::new()
            .add_pem(TEST_CERT_PEM)
            .expect("add pem")
            .add_file(&path)
            .expect("add file")
            .finish()
            .expect("finish");
        assert!(!store.is_empty());
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_add_source_file_variant() {
        let path = write_temp(TEST_CERT_PEM);
        let src = CaSource::File { path: path.clone() };
        let store = RootStoreBuilder::new()
            .add_source(&src)
            .await
            .expect("add source")
            .finish()
            .expect("finish");
        assert!(!store.is_empty());
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_add_source_pem_variant() {
        let src = CaSource::Pem {
            data: TEST_CERT_PEM.to_string(),
        };
        let store = RootStoreBuilder::new()
            .add_source(&src)
            .await
            .expect("add source")
            .finish()
            .expect("finish");
        assert!(!store.is_empty());
    }

    #[test]
    fn test_invalid_pem_returns_error() {
        let bad_pem = "-----BEGIN CERTIFICATE-----\nBAD!@#\n-----END CERTIFICATE-----";
        let result = RootStoreBuilder::new().add_pem(bad_pem);
        assert!(matches!(result, Err(ConfigError::InvalidPem(_))));
    }

    // We cannot reliably assert >0 for system roots in hermetic environments, just that it doesn't panic.
    #[test]
    fn test_with_system_roots_no_panic() {
        let _ = RootStoreBuilder::new().with_system_roots().finish();
    }
}
