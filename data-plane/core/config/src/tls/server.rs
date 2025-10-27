// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{path::Path, sync::Arc};

use rustls::{
    RootCertStore, ServerConfig as RustlsServerConfig,
    server::{ResolvesServerCert, WebPkiClientVerifier},
    version::{TLS12, TLS13},
};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};

use super::common::{Config, ConfigError, RustlsConfigLoader};
use crate::{
    component::configuration::{Configuration, ConfigurationError},
    tls::common::{SpireCertResolver, StaticCertResolver, WatcherCertResolver},
};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct TlsServerConfig {
    /// The Config struct
    #[serde(flatten, default)]
    pub config: Config,

    /// insecure do not setup a TLS server
    #[serde(default = "default_insecure")]
    pub insecure: bool,

    /// Path to the TLS cert to use by the server to verify a client certificate. (optional)
    pub client_ca_file: Option<String>,

    /// PEM encoded CA cert to use by the server to verify a client certificate. (optional)
    pub client_ca_pem: Option<String>,

    /// Reload the ClientCAs file when it is modified
    /// TODO(msardara): not implemented yet
    #[serde(default = "default_reload_client_ca_file")]
    pub reload_client_ca_file: bool,
}

impl Default for TlsServerConfig {
    fn default() -> Self {
        TlsServerConfig {
            config: Config::default(),
            insecure: default_insecure(),
            client_ca_file: None,
            client_ca_pem: None,
            reload_client_ca_file: default_reload_client_ca_file(),
        }
    }
}

fn default_insecure() -> bool {
    false
}

fn default_reload_client_ca_file() -> bool {
    false
}

/// Display the ServerConfig
impl std::fmt::Display for TlsServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl ResolvesServerCert for WatcherCertResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.cert.read().clone())
    }
}

impl ResolvesServerCert for StaticCertResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.cert.clone())
    }
}

impl ResolvesServerCert for SpireCertResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        match self.build_certified_key() {
            Ok((ck, _)) => Some(ck),
            Err(e) => {
                tracing::warn!(error=%e, "spire cert resolver: failed to build server cert");
                None
            }
        }
    }
}

// methods for ServerConfig to create a RustlsServerConfig from the config
impl TlsServerConfig {
    /// Create a new TlsServerConfig
    pub fn new() -> Self {
        TlsServerConfig {
            ..Default::default()
        }
    }

    /// Create insecure TlsServerConfig
    /// This will disable TLS and allow all connections
    pub fn insecure() -> Self {
        TlsServerConfig {
            insecure: true,
            ..Default::default()
        }
    }

    /// Set insecure (disable TLS)
    pub fn with_insecure(self, insecure: bool) -> Self {
        TlsServerConfig { insecure, ..self }
    }

    /// Set CA file for client auth
    pub fn with_client_ca_file(self, client_ca_file: &str) -> Self {
        TlsServerConfig {
            client_ca_file: Some(client_ca_file.to_string()),
            ..self
        }
    }

    /// Set CA pem for client auth
    pub fn with_client_ca_pem(self, client_ca_pem: &str) -> Self {
        TlsServerConfig {
            client_ca_pem: Some(client_ca_pem.to_string()),
            ..self
        }
    }

    /// Set reload_client_ca_file
    pub fn with_reload_client_ca_file(self, reload_client_ca_file: bool) -> Self {
        TlsServerConfig {
            reload_client_ca_file,
            ..self
        }
    }

    /// Set CA file
    pub fn with_ca_file(self, ca_file: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_ca_file(ca_file),
            ..self
        }
    }

    /// Set CA pem
    pub fn with_ca_pem(self, ca_pem: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_ca_pem(ca_pem),
            ..self
        }
    }

    /// Set include system CA certs pool
    pub fn with_include_system_ca_certs_pool(self, include_system_ca_certs_pool: bool) -> Self {
        TlsServerConfig {
            config: self
                .config
                .with_include_system_ca_certs_pool(include_system_ca_certs_pool),
            ..self
        }
    }

    /// Set cert file
    pub fn with_cert_file(self, cert_file: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_cert_file(cert_file),
            ..self
        }
    }

    /// Set cert pem
    pub fn with_cert_pem(self, cert_pem: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_cert_pem(cert_pem),
            ..self
        }
    }

    /// Set key file
    pub fn with_key_file(self, key_file: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_key_file(key_file),
            ..self
        }
    }

    /// Set key pem
    pub fn with_key_pem(self, key_pem: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_key_pem(key_pem),
            ..self
        }
    }

    /// Set TLS version
    pub fn with_tls_version(self, tls_version: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_tls_version(tls_version),
            ..self
        }
    }

    /// Set reload interval
    pub fn with_reload_interval(self, reload_interval: Option<std::time::Duration>) -> Self {
        TlsServerConfig {
            config: self.config.with_reload_interval(reload_interval),
            ..self
        }
    }

    /// Attach a SPIFFE configuration (SPIRE Workload API) for dynamic SVID + bundle based TLS.
    pub fn with_spiffe(self, spiffe: crate::auth::spiffe::SpiffeConfig) -> Self {
        TlsServerConfig {
            config: self.config.with_spiffe(spiffe),
            ..self
        }
    }

    pub fn load_rustls_server_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
        // SPIFFE/SPIRE configured => use async path (return error if called synchronously)
        if self.config.spire.is_some() {
            return Err(ConfigError::InvalidFile(
                "SPIFFE is configured; use async TLS loader".to_string(),
            ));
        }

        // Check if insecure is set
        if self.insecure {
            return Ok(None);
        }

        // Check TLS version
        let tls_version = match self.config.tls_version.as_str() {
            "tls1.2" => &TLS12,
            "tls1.3" => &TLS13,
            _ => {
                return Err(ConfigError::InvalidTlsVersion(
                    self.config.tls_version.clone(),
                ));
            }
        };

        // create a server ConfigBuilder
        let config_builder = RustlsServerConfig::builder_with_protocol_versions(&[tls_version]);

        // Get certificate & key
        let resolver: Arc<dyn ResolvesServerCert> = match (
            self.config.has_cert_file() && self.config.has_key_file(),
            self.config.has_cert_pem() && self.config.has_key_pem(),
        ) {
            (true, true) => {
                // If both cert_file and cert_pem are set, return an error
                return Err(ConfigError::CannotUseBoth("cert".to_string()));
            }
            (false, false) => {
                // If no cert, return an error
                return Err(ConfigError::MissingServerCertAndKey);
            }
            (true, false) => Arc::new(WatcherCertResolver::new(
                self.config.key_file.as_ref().unwrap(),
                self.config.cert_file.as_ref().unwrap(),
                config_builder.crypto_provider(),
            )?),
            (false, true) => Arc::new(StaticCertResolver::new(
                self.config.key_pem.as_ref().unwrap(),
                self.config.cert_pem.as_ref().unwrap(),
                config_builder.crypto_provider(),
            )?),
        };

        // Check whether to enable client auth or not
        let client_ca_certs = match (&self.client_ca_file, &self.client_ca_pem) {
            (Some(_), Some(_)) => return Err(ConfigError::CannotUseBoth("client_ca".to_string())),
            (Some(file_path), None) => {
                let certs: Result<Vec<_>, _> = CertificateDer::pem_file_iter(Path::new(file_path))
                    .map_err(ConfigError::InvalidPem)?
                    .collect();
                Some(certs.map_err(ConfigError::InvalidPem)?)
            }
            (None, Some(pem_data)) => {
                let certs: Result<Vec<_>, _> =
                    CertificateDer::pem_slice_iter(pem_data.as_bytes()).collect();
                Some(certs.map_err(ConfigError::InvalidPem)?)
            }
            (None, None) => None,
        };

        // create root store if client_ca_certs is set
        let server_config = match client_ca_certs {
            Some(client_ca_certs) => {
                let mut root_store = RootCertStore::empty();
                for cert in client_ca_certs {
                    root_store.add(cert).map_err(ConfigError::RootStore)?;
                }
                let verifier = WebPkiClientVerifier::builder(root_store.into())
                    .build()
                    .map_err(ConfigError::VerifierBuilder)?;
                config_builder
                    .with_client_cert_verifier(verifier)
                    .with_cert_resolver(resolver)
            }
            None => config_builder
                .with_no_client_auth()
                .with_cert_resolver(resolver),
        };

        // We are good to go
        Ok(Some(server_config))
    }

    /// Async variant supporting SPIFFE/SPIRE dynamic SVID + bundle.
    pub async fn load_rustls_server_config_async(
        &self,
    ) -> Result<Option<RustlsServerConfig>, ConfigError> {
        // If insecure skip TLS
        if self.insecure {
            return Ok(None);
        }

        // TLS version
        let tls_version = match self.config.tls_version.as_str() {
            "tls1.2" => &TLS12,
            "tls1.3" => &TLS13,
            _ => {
                return Err(ConfigError::InvalidTlsVersion(
                    self.config.tls_version.clone(),
                ));
            }
        };

        let config_builder = RustlsServerConfig::builder_with_protocol_versions(&[tls_version]);

        // SPIFFE resolver branch
        if let Some(spire_sources) = &self.config.spire {
            let spiffe_cfg = &spire_sources.spiffe;
            // Build provider
            // Build unified SPIFFE resolver directly from configuration (no external provider construction)
            let spire_resolver = super::common::SpireCertResolver::new(spiffe_cfg.clone())
                .await
                .map_err(|e| ConfigError::InvalidFile(e.to_string()))?;

            // Build client CA root store from SPIRE bundles (async). This SPIRE bundle
            // now acts as the implicit client_ca when SPIFFE is configured, so callers
            // do not need to set client_ca_file or client_ca_pem explicitly.
            //
            // Additionally, if caller still provided client_ca_file or client_ca_pem,
            // we merge those CAs into the SPIRE-derived root store (allowing mixed trust).
                        let mut root_store = self
                            .config
                            .load_ca_cert_pool_async()
                            .await
                            .map_err(|e| e)?;
                        // Merge explicit client CA sources if present (optional)
                        if let Some(ca_file) = &self.client_ca_file {
                            match CertificateDer::pem_file_iter(Path::new(ca_file)) {
                                Ok(iter) => {
                                    for cert_res in iter {
                                        match cert_res {
                                            Ok(cert) => {
                                                if let Err(e) = root_store.add(cert) {
                                                    tracing::warn!(error=%e, "failed adding client_ca_file cert");
                                                }
                                            }
                                            Err(e) => tracing::warn!(error=%e, "invalid PEM in client_ca_file"),
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!(error=%e, "failed reading client_ca_file"),
                            }
                        }
                        if let Some(ca_pem) = &self.client_ca_pem {
                            match CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
                                ok_iter => {
                                    for cert_res in ok_iter {
                                        match cert_res {
                                            Ok(cert) => {
                                                if let Err(e) = root_store.add(cert) {
                                                    tracing::warn!(error=%e, "failed adding client_ca_pem cert");
                                                }
                                            }
                                            Err(e) => tracing::warn!(error=%e, "invalid PEM in client_ca_pem"),
                                        }
                                    }
                                }
                            }
                        }
                        if root_store.is_empty() {
                            tracing::warn!(
                                "SPIRE + explicit client CA sources produced empty root store; client cert verification will fail"
                            );
                        }

            // If empty, attempt adding SVID intermediates (optional)
            if root_store.is_empty() {
                tracing::warn!("SPIFFE root store empty; client cert verification may fail");
            }

            let verifier = WebPkiClientVerifier::builder(root_store.into())
                .build()
                .map_err(ConfigError::VerifierBuilder)?;

            let server_config = config_builder
                .with_client_cert_verifier(verifier)
                .with_cert_resolver(Arc::new(spire_resolver));

            return Ok(Some(server_config));
        }

        // Fallback to synchronous path if no SPIFFE
        self.load_rustls_server_config()
    }
}

// trait implementation
#[async_trait::async_trait]
impl RustlsConfigLoader<RustlsServerConfig> for TlsServerConfig {
    async fn load_rustls_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
        if self.config.spire.is_some() {
            self.load_rustls_server_config_async().await
        } else {
            self.load_rustls_server_config()
        }
    }
}

impl Configuration for TlsServerConfig {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // TODO(msardara): validate the configuration
        Ok(())
    }
}
