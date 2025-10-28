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
    tls::common::{self, SpireCertResolver, StaticCertResolver, WatcherCertResolver},
};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCaSources {
    File {
        path: String,
    },
    Pem {
        data: String,
    },
    Spire {
        config: crate::auth::spiffe::SpiffeConfig,
    },
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
            client_ca: Some(ClientCaSources::File {
                path: client_ca_file.to_string(),
            }),
            ..self
        }
    }

    /// Set client CA from PEM
    pub fn with_client_ca_pem(self, client_ca_pem: &str) -> Self {
        TlsServerConfig {
            client_ca: Some(ClientCaSources::Pem {
                data: client_ca_pem.to_string(),
            }),
            ..self
        }
    }

    /// Set client CA from SPIFFE (SPIRE)
    pub fn with_client_ca_spire(self, client_ca_spire: crate::auth::spiffe::SpiffeConfig) -> Self {
        TlsServerConfig {
            client_ca: Some(ClientCaSources::Spire {
                config: client_ca_spire,
            }),
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
    pub async fn load_rustls_server_config(
        &self,
    ) -> Result<Option<RustlsServerConfig>, ConfigError> {
        use ConfigError::*;

        // Insecure => no TLS
        if self.insecure {
            return Ok(None);
        }

        // TLS version
        let tls_version = match self.config.tls_version.as_str() {
            "tls1.2" => &TLS12,
            "tls1.3" => &TLS13,
            _ => return Err(InvalidTlsVersion(self.config.tls_version.clone())),
        };

        let config_builder = RustlsServerConfig::builder_with_protocol_versions(&[tls_version]);

        // Unified handling based on TlsSource enum
        let resolver: Arc<dyn ResolvesServerCert> = match &self.config.source {
            // SPIFFE server cert path (dynamic SVID)
            Some(super::common::TlsSource::Spire { config: spiffe_cfg }) => {
                let spire_resolver =
                    SpireCertResolver::new(spiffe_cfg.clone(), config_builder.crypto_provider())
                        .await
                        .map_err(|e| ConfigError::InvalidFile(e.to_string()))?;

                // Build client CA root store (start with async CA bundle including SPIRE trust bundle)
                let mut root_store = self.config.load_ca_cert_pool().await?;

                // Merge client CA sources if provided (file/pem/spire)
                if let Some(client_ca_variant) = &self.client_ca {
                    match client_ca_variant {
                        ClientCaSources::File { path } => {
                            if let Ok(iter) = CertificateDer::pem_file_iter(Path::new(path)) {
                                for cert in iter.flatten() {
                                    let _ = root_store.add(cert);
                                }
                            }
                        }
                        ClientCaSources::Pem { data } => {
                            for cert in CertificateDer::pem_slice_iter(data.as_bytes()).flatten() {
                                let _ = root_store.add(cert);
                            }
                        }
                        ClientCaSources::Spire {
                            config: client_spiffe_cfg,
                        } => {
                            common::add_spiffe_bundles(&mut root_store, client_spiffe_cfg)
                                .await
                                .map_err(|e| {
                                    ConfigError::Spire(format!(
                                        "Failed to load SPIRE client CA bundle: {}",
                                        e
                                    ))
                                })?;
                        }
                    }
                }

                let verifier = WebPkiClientVerifier::builder(root_store.into())
                    .build()
                    .map_err(ConfigError::VerifierBuilder)?;

                let server_config = config_builder
                    .with_client_cert_verifier(verifier)
                    .with_cert_resolver(Arc::new(spire_resolver));

                return Ok(Some(server_config));
            }

            // Static file-based certificates
            Some(super::common::TlsSource::File { cert, key, .. }) => match (cert, key) {
                (Some(cert_path), Some(key_path)) => Arc::new(WatcherCertResolver::new(
                    key_path,
                    cert_path,
                    config_builder.crypto_provider(),
                )?),
                (None, None) => return Err(MissingServerCertAndKey),
                _ => return Err(MissingServerCertAndKey),
            },

            // Static PEM-based certificates
            Some(super::common::TlsSource::Pem { cert, key, .. }) => match (cert, key) {
                (Some(cert_pem), Some(key_pem)) => Arc::new(StaticCertResolver::new(
                    key_pem,
                    cert_pem,
                    config_builder.crypto_provider(),
                )?),
                (None, None) => return Err(MissingServerCertAndKey),
                _ => return Err(MissingServerCertAndKey),
            },

            // No source configured
            None => return Err(MissingServerCertAndKey),
        };

        // Build optional client auth verifier (non-SPIFFE server cert path)
        let client_ca_variant = self.client_ca.as_ref();

        // Collect client CA certificates based on variant (single source)
        let maybe_root_store = match client_ca_variant {
            Some(ClientCaSources::File { path }) => {
                let mut rs = RootCertStore::empty();
                let certs: Result<Vec<_>, _> = CertificateDer::pem_file_iter(Path::new(path))
                    .map_err(ConfigError::InvalidPem)?
                    .collect();
                for cert in certs.map_err(ConfigError::InvalidPem)? {
                    rs.add(cert).map_err(ConfigError::RootStore)?;
                }
                Some(rs)
            }
            Some(ClientCaSources::Pem { data }) => {
                let mut rs = RootCertStore::empty();
                let certs: Result<Vec<_>, _> =
                    CertificateDer::pem_slice_iter(data.as_bytes()).collect();
                for cert in certs.map_err(ConfigError::InvalidPem)? {
                    rs.add(cert).map_err(ConfigError::RootStore)?;
                }
                Some(rs)
            }
            Some(ClientCaSources::Spire { config: spiffe_cfg }) => {
                // Load SPIRE bundle for client auth
                if let Ok(spire_resolver) =
                    SpireCertResolver::new(spiffe_cfg.clone(), config_builder.crypto_provider())
                        .await
                {
                    if let Ok(bundle) = spire_resolver.load_ca_bundle() {
                        let mut rs = RootCertStore::empty();
                        for cert in bundle {
                            let _ = rs.add(cert);
                        }
                        Some(rs)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            None => None,
        };

        let server_config = if let Some(root_store) = maybe_root_store {
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
