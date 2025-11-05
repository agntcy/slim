// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use rustls::{
    ServerConfig as RustlsServerConfig,
    server::{NoClientAuth, ResolvesServerCert, WebPkiClientVerifier, danger::ClientCertVerifier},
    version::{TLS12, TLS13},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{Config, ConfigError, RustlsConfigLoader, TlsSource};
use crate::{
    component::configuration::{Configuration, ConfigurationError},
    tls::{
        RootStoreBuilder,
        common::{CaSource, StaticCertResolver, WatcherCertResolver},
    },
};

#[cfg(not(target_family = "windows"))]
use crate::tls::common::SpireCertResolver;

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
pub struct TlsServerConfig {
    /// The Config struct
    #[serde(flatten, default)]
    pub config: Config,

    /// insecure do not setup a TLS server
    #[serde(default = "default_insecure")]
    pub insecure: bool,

    /// Client CA sources (choose one: file, pem, or spire for SPIFFE bundle)
    #[serde(default)]
    pub client_ca: CaSource,

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
            client_ca: CaSource::default(),
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
            client_ca: CaSource::File {
                path: client_ca_file.to_string(),
            },
            ..self
        }
    }

    /// Set client CA from PEM
    pub fn with_client_ca_pem(self, client_ca_pem: &str) -> Self {
        TlsServerConfig {
            client_ca: CaSource::Pem {
                data: client_ca_pem.to_string(),
            },
            ..self
        }
    }

    /// Set client CA from SPIRE
    #[cfg(not(target_family = "windows"))]
    pub fn with_client_ca_spire(self, client_ca_spire: crate::auth::spire::SpireConfig) -> Self {
        TlsServerConfig {
            client_ca: CaSource::Spire {
                config: client_ca_spire,
            },
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

    /// Set cert and key file
    pub fn with_cert_and_key_file(self, cert_path: &str, key_path: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_cert_and_key_file(cert_path, key_path),
            ..self
        }
    }

    /// Set cert and key pem
    pub fn with_cert_and_key_pem(self, cert_pem: &str, key_pem: &str) -> Self {
        TlsServerConfig {
            config: self.config.with_cert_and_key_pem(cert_pem, key_pem),
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

    /// Attach a SPIRE configuration (SPIRE Workload API) for dynamic SVID + bundle based TLS.
    #[cfg(not(target_family = "windows"))]
    pub fn with_spire(self, spire: crate::auth::spire::SpireConfig) -> Self {
        TlsServerConfig {
            config: self.config.with_spire(spire),
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
            // SPIRE server path
            #[cfg(not(target_family = "windows"))]
            TlsSource::Spire { config: spire_cfg } => Arc::new(
                SpireCertResolver::new(spire_cfg.clone(), config_builder.crypto_provider())
                    .await
                    .map_err(|e| ConfigError::InvalidSpireConfig {
                        details: e.to_string(),
                        config: spire_cfg.clone(),
                    })?,
            ),
            // Static file-based certificates
            TlsSource::File { cert, key, .. } => Arc::new(WatcherCertResolver::new(
                key,
                cert,
                config_builder.crypto_provider(),
            )?),
            // Static PEM-based certificates
            TlsSource::Pem { cert, key, .. } => Arc::new(StaticCertResolver::new(
                key,
                cert,
                config_builder.crypto_provider(),
            )?),
            // No source configured
            TlsSource::None => return Err(MissingServerCertAndKey),
        };

        let client_verifier: Arc<dyn ClientCertVerifier> = match &self.client_ca {
            CaSource::None => Arc::new(NoClientAuth {}),
            _ => {
                // Build optional client auth verifier
                let root_store = RootStoreBuilder::new()
                    .add_source(&self.client_ca)
                    .await?
                    .finish()?;

                WebPkiClientVerifier::builder(root_store.into())
                    .build()
                    .map_err(ConfigError::VerifierBuilder)?
            }
        };

        let server_config = config_builder
            .with_client_cert_verifier(client_verifier)
            .with_cert_resolver(resolver);

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
