// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use rustls::{
    RootCertStore, ServerConfig as RustlsServerConfig,
    server::WebPkiClientVerifier,
    version::{TLS12, TLS13},
};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;

use super::common::{Config, ConfigError, RustlsConfigLoader};
use crate::component::configuration::{Configuration, ConfigurationError};

#[derive(Debug, Deserialize, PartialEq, Clone)]
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

// methods for ServerConfig to create a RustlsServerConfig from the config
impl TlsServerConfig {
    /// Create a new TlsServerConfig
    pub fn new() -> Self {
        TlsServerConfig {
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

    pub fn load_rustls_server_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
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

        // Get certificate & key
        let (cert, key) = match (
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
            (true, false) => (
                CertificateDer::from_pem_file(Path::new(self.config.cert_file.as_ref().unwrap()))
                    .map_err(ConfigError::InvalidPem)?,
                PrivateKeyDer::from_pem_file(Path::new(self.config.key_file.as_ref().unwrap()))
                    .map_err(ConfigError::InvalidPem)?,
            ),
            (false, true) => (
                CertificateDer::from_pem_slice(self.config.cert_pem.as_ref().unwrap().as_bytes())
                    .map_err(ConfigError::InvalidPem)?,
                PrivateKeyDer::from_pem_slice(self.config.key_pem.as_ref().unwrap().as_bytes())
                    .map_err(ConfigError::InvalidPem)?,
            ),
        };

        // create a server ConfigBuilder
        let config_builder = RustlsServerConfig::builder_with_protocol_versions(&[tls_version]);

        // Check whether to enable client auth or not
        let client_ca = match (&self.client_ca_file, &self.client_ca_pem) {
            (Some(_), Some(_)) => return Err(ConfigError::CannotUseBoth("client_ca".to_string())),
            (Some(_), None) => Option::Some(
                CertificateDer::from_pem_file(Path::new(self.client_ca_file.as_ref().unwrap()))
                    .map_err(ConfigError::InvalidPem)?,
            ),
            (None, Some(_)) => Option::Some(
                CertificateDer::from_pem_slice(self.client_ca_pem.as_ref().unwrap().as_bytes())
                    .map_err(ConfigError::InvalidPem)?,
            ),
            (None, None) => Option::None,
        };

        // create root store if client_ca is set
        let server_config = match client_ca {
            Some(client_ca) => {
                let mut root_store = RootCertStore::empty();
                root_store.add(client_ca).map_err(ConfigError::RootStore)?;
                let verifier = WebPkiClientVerifier::builder(root_store.into())
                    .build()
                    .map_err(ConfigError::VerifierBuilder)?;
                config_builder
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(vec![cert], key)
                    .map_err(ConfigError::ConfigBuilder)?
            }
            None => config_builder
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .map_err(ConfigError::ConfigBuilder)?,
        };

        // We are good to go
        Ok(Some(server_config))
    }
}

// trait implementation
impl RustlsConfigLoader<RustlsServerConfig> for TlsServerConfig {
    fn load_rustls_config(&self) -> Result<Option<RustlsServerConfig>, ConfigError> {
        let server_config = self.load_rustls_server_config()?;
        Ok(server_config)
    }
}

impl Configuration for TlsServerConfig {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // TODO(msardara): validate the configuration
        Ok(())
    }
}
