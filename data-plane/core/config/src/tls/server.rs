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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCaSources {
    File { path: String },
    Pem { data: String },
    Spire { config: crate::auth::spiffe::SpiffeConfig },
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct TlsServerConfig {
    /// The Config struct
    #[serde(flatten, default)]
    pub config: Config,

    /// insecure do not setup a TLS server
    #[serde(default = "default_insecure")]
    pub insecure: bool,

    /// Client CA sources (choose one: file, pem, or spire for SPIFFE bundle)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ca: Option<ClientCaSources>,

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
            client_ca: None,
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

    /// Set client CA from file
    pub fn with_client_ca_file(self, client_ca_file: &str) -> Self {
        TlsServerConfig {
            client_ca: Some(ClientCaSources::File { path: client_ca_file.to_string() }),
            ..self
        }
    }

    /// Set client CA from PEM
    pub fn with_client_ca_pem(self, client_ca_pem: &str) -> Self {
        TlsServerConfig {
            client_ca: Some(ClientCaSources::Pem { data: client_ca_pem.to_string() }),
            ..self
        }
    }

    /// Set client CA from SPIFFE (SPIRE)
    pub fn with_client_ca_spire(self, client_ca_spire: crate::auth::spiffe::SpiffeConfig) -> Self {
        TlsServerConfig {
            client_ca: Some(ClientCaSources::Spire { config: client_ca_spire }),
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

    /// Unified async loader supporting legacy file/pem certs and SPIFFE/SPIRE dynamic SVID.
    pub async fn load_rustls_server_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
        // Insecure => no TLS
        if self.insecure {
            return Ok(None);
        }

        // TLS version
        let tls_version = match self.config.tls_version.as_str() {
            "tls1.2" => &TLS12,
            "tls1.3" => &TLS13,
            _ => return Err(ConfigError::InvalidTlsVersion(self.config.tls_version.clone())),
        };

        let config_builder = RustlsServerConfig::builder_with_protocol_versions(&[tls_version]);

        // SPIFFE/SPIRE branch (dynamic server cert + bundled client CA verification)
        if let Some(spire_sources) = &self.config.spire {
            let spiffe_cfg = &spire_sources.spiffe;
            // Extract client CA enum variant (if any)
            let client_ca_variant = self.client_ca.as_ref();

            // Dynamic resolver for server SVID
            let spire_resolver = super::common::SpireCertResolver::new(spiffe_cfg.clone())
                .await
                .map_err(|e| ConfigError::InvalidFile(e.to_string()))?;

            // Start with SPIRE CA bundle (server spire config)
            let mut root_store = self
                .config
                .load_ca_cert_pool_async()
                .await
                .map_err(|e| e)?;

            // Optionally merge a distinct SPIFFE client CA source (client_ca_spire)
            if let Some(ClientCaSources::Spire { config: client_spiffe_cfg }) = client_ca_variant {
                if let Ok(client_spire_resolver) = SpireCertResolver::new(client_spiffe_cfg.clone()).await {
                    if let Ok(bundle) = client_spire_resolver.load_ca_bundle() {
                        for cert in bundle {
                            let _ = root_store.add(cert);
                        }
                    }
                }
            }

            // Merge explicit client CA (file)
            if let Some(ClientCaSources::File { path: ca_file }) = client_ca_variant {
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

            // Merge explicit client CA (pem)
            if let Some(ClientCaSources::Pem { data: ca_pem }) = client_ca_variant {
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

            let verifier = WebPkiClientVerifier::builder(root_store.into())
                .build()
                .map_err(ConfigError::VerifierBuilder)?;

            let server_config = config_builder
                .with_client_cert_verifier(verifier)
                .with_cert_resolver(Arc::new(spire_resolver));

            return Ok(Some(server_config));
        }

        // Legacy file/pem branch
        let resolver: Arc<dyn ResolvesServerCert> = match (
            self.config.has_cert_file() && self.config.has_key_file(),
            self.config.has_cert_pem() && self.config.has_key_pem(),
        ) {
            (true, true) => return Err(ConfigError::CannotUseBoth("cert".to_string())),
            (false, false) => return Err(ConfigError::MissingServerCertAndKey),
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

        // Optional client auth root store
        // Enum variant access
        let client_ca_variant = self.client_ca.as_ref();

        let client_ca_certs = match client_ca_variant {
            Some(ClientCaSources::File { path }) => {
                let certs: Result<Vec<_>, _> = CertificateDer::pem_file_iter(Path::new(path))
                    .map_err(ConfigError::InvalidPem)?
                    .collect();
                Some(certs.map_err(ConfigError::InvalidPem)?)
            }
            Some(ClientCaSources::Pem { data }) => {
                let certs: Result<Vec<_>, _> =
                    CertificateDer::pem_slice_iter(data.as_bytes()).collect();
                Some(certs.map_err(ConfigError::InvalidPem)?)
            }
            _ => None, // Spire handled separately; None means no explicit file/pem
        };

        let server_config = match client_ca_certs {
            Some(client_ca_certs) => {
                let mut root_store = RootCertStore::empty();
                for cert in client_ca_certs {
                    root_store.add(cert).map_err(ConfigError::RootStore)?;
                }
                // Merge SPIFFE client CA bundle if client_ca_spire is set
                if let Some(ClientCaSources::Spire { config: spiffe_cfg }) = client_ca_variant {
                    if let Ok(spire_resolver) = SpireCertResolver::new(spiffe_cfg.clone()).await {
                        if let Ok(bundle) = spire_resolver.load_ca_bundle() {
                            for cert in bundle {
                                let _ = root_store.add(cert);
                            }
                        }
                    }
                }
                let verifier = WebPkiClientVerifier::builder(root_store.into())
                    .build()
                    .map_err(ConfigError::VerifierBuilder)?;
                config_builder
                    .with_client_cert_verifier(verifier)
                    .with_cert_resolver(resolver)
            }
            None => {
                // If a SPIFFE client CA source is provided, enable client auth with its bundle
                if let Some(ClientCaSources::Spire { config: spiffe_cfg }) = client_ca_variant {
                    let mut root_store = RootCertStore::empty();
                    if let Ok(spire_resolver) = SpireCertResolver::new(spiffe_cfg.clone()).await {
                        if let Ok(bundle) = spire_resolver.load_ca_bundle() {
                            for cert in bundle {
                                let _ = root_store.add(cert);
                            }
                        }
                    }
                    let verifier = WebPkiClientVerifier::builder(root_store.into())
                        .build()
                        .map_err(ConfigError::VerifierBuilder)?;
                    config_builder
                        .with_client_cert_verifier(verifier)
                        .with_cert_resolver(resolver)
                } else {
                    config_builder
                        .with_no_client_auth()
                        .with_cert_resolver(resolver)
                }
            }
        };

        Ok(Some(server_config))
    }
}

// trait implementation
#[async_trait::async_trait]
impl RustlsConfigLoader<RustlsServerConfig> for TlsServerConfig {
    async fn load_rustls_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
        self.load_rustls_server_config().await
    }
}

impl Configuration for TlsServerConfig {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // TODO(msardara): validate the configuration
        Ok(())
    }
}
