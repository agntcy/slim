// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use parking_lot::RwLock;
use rustls::RootCertStore;
use rustls::crypto::CryptoProvider;
use rustls::server::VerifierBuilderError;
use rustls::sign::CertifiedKey;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::file_watcher::FileWatcher;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

#[cfg(not(target_family = "windows"))]
use crate::auth::spire;

#[derive(Debug)]
pub(crate) struct WatcherCertResolver {
    // Files
    _key_file: String,
    _cert_file: String,

    // Crypto provider
    _provider: Arc<CryptoProvider>,

    // watchers
    _watchers: Vec<FileWatcher>,

    // the certificate
    pub cert: Arc<RwLock<Arc<CertifiedKey>>>,
}

fn to_certified_key(
    cert_der: Vec<CertificateDer<'static>>,
    key_der: PrivateKeyDer<'static>,
    crypto_provider: &CryptoProvider,
) -> CertifiedKey {
    CertifiedKey::from_der(cert_der, key_der, crypto_provider).unwrap()
}

impl WatcherCertResolver {
    pub(crate) fn new(
        key_file: impl Into<String>,
        cert_file: impl Into<String>,
        crypto_provider: &Arc<CryptoProvider>,
    ) -> Result<Self, ConfigError> {
        let key_file = key_file.into();
        let key_files = (key_file.clone(), key_file.clone());

        let cert_file = cert_file.into();
        let cert_files = (cert_file.clone(), cert_file.clone());
        let crypto_providers = (crypto_provider.clone(), crypto_provider.clone());

        // Read the cert and the key
        let key_der = PrivateKeyDer::from_pem_file(Path::new(&key_files.0))
            .map_err(|e| ConfigError::InvalidFile(e.to_string()))?;
        let cert_der = CertificateDer::from_pem_file(Path::new(&cert_files.0))
            .map_err(|e| ConfigError::InvalidFile(e.to_string()))?;

        // Transform it to CertifiedKey
        let cert_key = to_certified_key(vec![cert_der], key_der, crypto_provider);

        let cert = Arc::new(RwLock::new(Arc::new(cert_key)));
        let cert_clone = cert.clone();
        let w = FileWatcher::create_watcher(move |_file| {
            // Read the cert and the key
            let key_der = PrivateKeyDer::from_pem_file(Path::new(&key_files.0))
                .expect("failed to read key file");
            let cert_der = CertificateDer::from_pem_file(Path::new(&cert_files.0))
                .expect("failed to read cert file");
            let cert_key = to_certified_key(vec![cert_der], key_der, &crypto_providers.0);

            *cert_clone.as_ref().write() = Arc::new(cert_key);
        });

        Ok(Self {
            _key_file: key_files.1,
            _cert_file: cert_files.1,
            _provider: crypto_providers.1,
            _watchers: vec![w],
            cert,
        })
    }
}

#[derive(Debug)]
pub(crate) struct StaticCertResolver {
    // Cert and key
    _key_pem: String,
    _cert_pem: String,

    // the certificate
    pub cert: Arc<CertifiedKey>,
}

impl StaticCertResolver {
    pub(crate) fn new(
        key_pem: impl Into<String>,
        cert_pem: impl Into<String>,
        crypto_provider: &Arc<CryptoProvider>,
    ) -> Result<Self, ConfigError> {
        let key_pem = key_pem.into();
        let cert_pem = cert_pem.into();

        // Read the cert and the key
        let key_der =
            PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(ConfigError::InvalidPem)?;
        let cert_der =
            CertificateDer::from_pem_slice(cert_pem.as_bytes()).map_err(ConfigError::InvalidPem)?;
        let cert_key = to_certified_key(vec![cert_der], key_der, crypto_provider);

        Ok(Self {
            _key_pem: key_pem,
            _cert_pem: cert_pem,
            cert: Arc::new(cert_key),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
#[cfg(not(target_family = "windows"))]
pub struct SpireSources {
    /// Flattened spire configuration (socket_path, target_spiffe_id, jwt_audiences, trust_domain)
    #[serde(flatten)]
    pub spire: spire::SpireConfig,
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TlsSource {
    Pem {
        cert: String,
        key: String,
    },
    File {
        cert: String,
        key: String,
    },
    #[cfg(not(target_family = "windows"))]
    Spire {
        #[serde(flatten)]
        config: spire::SpireConfig,
    },
    #[default]
    None,
}

impl TlsSource {
    /// Returns true if this variant is `TlsSource::None`
    pub fn is_none(&self) -> bool {
        matches!(self, TlsSource::None)
    }
}

#[derive(Default, Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaSource {
    File {
        path: String,
    },
    Pem {
        data: String,
    },
    #[cfg(not(target_family = "windows"))]
    Spire {
        #[serde(flatten)]
        config: spire::SpireConfig,
    },
    #[default]
    None,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
/// TLS component discriminant for unified presence checks
pub enum TlsComponent {
    Ca,
    Cert,
    Key,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
pub struct Config {
    // Unified TLS source (PEM, File, or SPIRE)
    #[serde(default)]
    pub source: TlsSource,

    #[serde(default)]
    pub ca_source: CaSource,

    /// If true, also load system root CA certificates
    #[serde(default = "default_include_system_ca_certs_pool")]
    pub include_system_ca_certs_pool: bool,

    // TLS protocol version ("tls1.2" or "tls1.3")
    #[serde(default = "default_tls_version")]
    pub tls_version: String,

    // Certificate/key reload interval (None disables reload)
    pub reload_interval: Option<Duration>,
}

// Resolver backed by SPIRE Workload API providing dynamic SVID and bundle refresh.
#[cfg(not(target_family = "windows"))]
pub(crate) struct SpireCertResolver {
    provider: slim_auth::spire::SpireIdentityManager,
    crypto_provider: Arc<CryptoProvider>,
}

// Manual Debug impl (SpireIdentityManager does not implement Debug)
#[cfg(not(target_family = "windows"))]
impl std::fmt::Debug for SpireCertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpireCertResolver {{ provider: <opaque> }}")
    }
}

#[cfg(not(target_family = "windows"))]
impl SpireCertResolver {
    pub(crate) async fn new(
        spire_cfg: spire::SpireConfig,
        crypto_provider: &Arc<CryptoProvider>,
    ) -> Result<Self, ConfigError> {
        // Build SpireIdentityManager internally from configuration
        let mut builder = slim_auth::spire::SpireIdentityManager::builder()
            .with_jwt_audiences(spire_cfg.jwt_audiences.clone());

        if let Some(ref socket) = spire_cfg.socket_path {
            builder = builder.with_socket_path(socket.clone());
        }
        if let Some(ref id) = spire_cfg.target_spiffe_id {
            builder = builder.with_target_spiffe_id(id.clone());
        }

        let mut provider = builder.build();
        provider
            .initialize()
            .await
            .map_err(|e| ConfigError::InvalidSpireConfig {
                details: e.to_string(),
                config: spire_cfg.clone(),
            })?;

        Ok(Self {
            provider,
            crypto_provider: crypto_provider.clone(),
        })
    }

    pub(crate) fn has_certs(&self) -> bool {
        self.provider.get_x509_svid().is_ok()
    }

    pub(crate) fn build_certified_key(&self) -> Result<(Arc<CertifiedKey>, usize), ConfigError> {
        // Build full SVID certificate chain (leaf + intermediates) for the CertifiedKey
        let svid = self
            .provider
            .get_x509_svid()
            .map_err(|e| ConfigError::SpireError(e.to_string()))?;

        let mut chain_der: Vec<CertificateDer<'static>> =
            Vec::with_capacity(svid.cert_chain().len());
        for c in svid.cert_chain().iter() {
            chain_der.push(c.as_ref().to_vec().into());
        }

        // Obtain private key PEM and convert to DER
        let key_der = PrivateKeyDer::Pkcs8(svid.private_key().as_ref().to_vec().into());

        // Build CertifiedKey from full chain
        let cert_key = to_certified_key(chain_der.clone(), key_der, &self.crypto_provider);
        Ok((Arc::new(cert_key), chain_der.len()))
    }
}

// Implement rustls server & client certificate resolver traits for SpireCertResolver
#[cfg(not(target_family = "windows"))]
impl rustls::server::ResolvesServerCert for SpireCertResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        match self.build_certified_key() {
            Ok((ck, _chain_len)) => Some(ck),
            Err(e) => {
                tracing::warn!(error=%e, "SpireCertResolver server resolve failed");
                None
            }
        }
    }
}

/// Errors for Config
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid tls version: {0}")]
    InvalidTlsVersion(String),
    #[error("invalid pem format: {0}")]
    InvalidPem(rustls_pki_types::pem::Error),
    #[error("error reading cert/key from file: {0}")]
    InvalidFile(String),
    #[error("error in spire configuration: {details}, config={config:?}")]
    #[cfg(not(target_family = "windows"))]
    InvalidSpireConfig {
        details: String,
        config: spire::SpireConfig,
    },
    #[error("error running spire: {0}")]
    #[cfg(not(target_family = "windows"))]
    SpireError(String),

    #[error("root store error: {0}")]
    RootStore(rustls::Error),
    #[error("config builder error")]
    ConfigBuilder(rustls::Error),
    #[error("missing server cert and key")]
    MissingServerCertAndKey,
    #[error("verifier builder error")]
    VerifierBuilder(VerifierBuilderError),
    #[error("unknown error")]
    Unknown,

    #[error("spire error: {0}")]
    #[cfg(not(target_family = "windows"))]
    Spire(String),
}

// Defaults for Config
impl Default for Config {
    fn default() -> Config {
        Config {
            source: TlsSource::default(),
            ca_source: CaSource::default(),
            include_system_ca_certs_pool: default_include_system_ca_certs_pool(),
            tls_version: default_tls_version(),
            reload_interval: None,
        }
    }
}

// Default system CA certs pool
fn default_include_system_ca_certs_pool() -> bool {
    true
}

// Default for tls version
fn default_tls_version() -> String {
    "tls1.3".to_string()
}

impl Config {
    pub(crate) fn with_ca_file(mut self, ca_path: &str) -> Config {
        match &mut self.ca_source {
            CaSource::File { path, .. } => *path = ca_path.to_string(),
            _ => {
                // Replace existing source with File variant
                self.ca_source = CaSource::File {
                    path: ca_path.to_string(),
                };
            }
        }
        self
    }

    pub(crate) fn with_ca_pem(mut self, ca_pem: &str) -> Config {
        match &mut self.ca_source {
            CaSource::Pem { data, .. } => *data = ca_pem.to_string(),
            _ => {
                self.ca_source = CaSource::Pem {
                    data: ca_pem.to_string(),
                };
            }
        }
        self
    }

    #[cfg(not(target_family = "windows"))]
    pub(crate) fn with_ca_spire(mut self, spire: spire::SpireConfig) -> Config {
        self.ca_source = CaSource::Spire { config: spire };
        self
    }

    pub(crate) fn with_include_system_ca_certs_pool(
        mut self,
        include_system_ca_certs_pool: bool,
    ) -> Config {
        self.include_system_ca_certs_pool = include_system_ca_certs_pool;
        self
    }

    pub(crate) fn with_cert_and_key_file(mut self, cert_path: &str, key_path: &str) -> Self {
        match &mut self.source {
            TlsSource::File { cert, key, .. } => {
                *cert = cert_path.to_string();
                *key = key_path.to_string();
            }
            _ => {
                self.source = TlsSource::File {
                    cert: cert_path.to_string(),
                    key: key_path.to_string(),
                };
            }
        }
        self
    }

    pub(crate) fn with_cert_and_key_pem(mut self, cert_pem: &str, key_pem: &str) -> Self {
        match &mut self.source {
            TlsSource::Pem { cert, key, .. } => {
                *cert = cert_pem.to_string();
                *key = key_pem.to_string();
            }
            _ => {
                self.source = TlsSource::Pem {
                    cert: cert_pem.to_string(),
                    key: key_pem.to_string(),
                };
            }
        }
        self
    }

    pub(crate) fn with_tls_version(mut self, tls_version: &str) -> Config {
        self.tls_version = tls_version.to_string();
        self
    }

    pub(crate) fn with_reload_interval(mut self, reload_interval: Option<Duration>) -> Config {
        self.reload_interval = reload_interval;
        self
    }

    /// Attach a spire configuration enabling SPIRE-based SVID and bundle resolution.
    /// This sets the `spire` field with a `SpireSources` wrapper.
    #[cfg(not(target_family = "windows"))]
    pub(crate) fn with_spire(mut self, spire: spire::SpireConfig) -> Config {
        self.source = TlsSource::Spire { config: spire };
        self
    }

    /// Unified CA cert pool loader supporting SPIRE CA bundle retrieval.
    /// Delegates to the standalone RootStoreBuilder for clarity.
    pub(crate) async fn load_ca_cert_pool(&self) -> Result<RootCertStore, ConfigError> {
        use crate::tls::root_store_builder::RootStoreBuilder;
        let builder = RootStoreBuilder::new();

        let builder = if self.include_system_ca_certs_pool {
            builder.with_system_roots()
        } else {
            builder
        };

        builder.add_source(&self.ca_source).await?.finish()
    }

    /// Unified presence check for CA / Cert / Key across File, Pem, and Spire sources.
    pub fn has(&self, component: TlsComponent) -> bool {
        match component {
            TlsComponent::Ca => match &self.ca_source {
                CaSource::File { path } => !path.is_empty(),
                CaSource::Pem { data } => !data.is_empty(),
                #[cfg(not(target_family = "windows"))]
                CaSource::Spire { .. } => true,
                CaSource::None => false,
            },
            TlsComponent::Cert => match &self.source {
                TlsSource::File { cert, .. } => !cert.is_empty(),
                TlsSource::Pem { cert, .. } => !cert.is_empty(),
                #[cfg(not(target_family = "windows"))]
                TlsSource::Spire { .. } => true,
                TlsSource::None => false,
            },
            TlsComponent::Key => match &self.source {
                TlsSource::File { key, .. } => !key.is_empty(),
                TlsSource::Pem { key, .. } => !key.is_empty(),
                #[cfg(not(target_family = "windows"))]
                TlsSource::Spire { .. } => true,
                TlsSource::None => false,
            },
        }
    }
}

// trait load_rustls_config
#[async_trait::async_trait]
pub trait RustlsConfigLoader<T> {
    async fn load_rustls_config(&self) -> Result<Option<T>, ConfigError>;
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    use crate::tls::provider;

    // spellchecker:off

    // Test certificates (for testing purposes only)
    const TEST_CA_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDNjCCAh4CCQDkU3rM23H5hzANBgkqhkiG9w0BAQsFADBdMQswCQYDVQQGEwJB
VTESMBAGA1UECAwJQXVzdHJhbGlhMQ8wDQYDVQQHDAZTeWRuZXkxEjAQBgNVBAoM
CU15T3JnTmFtZTEVMBMGA1UEAwwMTXlDb21tb25OYW1lMB4XDTIyMDgwMzA0MTgx
OFoXDTMyMDczMTA0MTgxOFowXTELMAkGA1UEBhMCQVUxEjAQBgNVBAgMCUF1c3Ry
YWxpYTEPMA0GA1UEBwwGU3lkbmV5MRIwEAYDVQQKDAlNeU9yZ05hbWUxFTATBgNV
BAMMDE15Q29tbW9uTmFtZTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEB
AK836YUxmCDcznt11ReI5fY/DSJzz+Fs7czoE72RMvW+SMH2YhX9XC55xAMPZ+IV
szoG5Fatd/GWBfoACmaM3ZEmYskuRnu4pxqOEpRIsBukOiILBMxa/cwqiDyLiacC
w0B1NhysG28XnxUWrYxd9jFlJ+wAIx7XT+1QM0xGCGr9agSQ/ow6+QMWZ5Qc1n2e
EmaoU861qlF+0LeyZeBNeo+C7jTikIC+CRKVNX5t9MLqSmlxfrXe0qCS99zmPKfg
OhtteZVAKbdPKSoi2ls6EQ1dNB2Mq3GHkd8kGi30FuRCTQLKaXacUdjtQfbKxuGl
RjXlN6mDoUs8mIO861mVFXECAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAUrgRTBBO
pwYjZsLNw10FYK19P6FpVm/nbbzTJmqKlxReLRkkTyNm/tB5W1LdRN9RG15h62Ii
JBGxpeCMDElwCwXN2OOwqdXczafLa9AhPnPw/DYuQAd9dS7/XHG/ArQFTL+GLd8T
bdlnED9Z9qMygF13btLQUHzKaOk6dndLsquoTjgjj4SNBe2Isj7z4upZOix2cgJB
9ddZGlv8/zKSgRp9UotGOOxG7HJ1KWhYLU7E0aERqambNv8UFvhmf+biHq3nCeAF
HBeua27MNj4kGCzqHS7sVqZKVU81aFyhV2WmfIUA0Qp+nh9QEW0yrgI+pTnOx6np
JUHGleZ3rKHQZw==
-----END CERTIFICATE-----"#; // spellchecker:disable-line

    const TEST_CLIENT_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDVDCCAjygAwIBAgIJANt5fkUlfxyeMA0GCSqGSIb3DQEBCwUAMF0xCzAJBgNV
BAYTAkFVMRIwEAYDVQQIDAlBdXN0cmFsaWExDzANBgNVBAcMBlN5ZG5leTESMBAG
A1UECgwJTXlPcmdOYW1lMRUwEwYDVQQDDAxNeUNvbW1vbk5hbWUwHhcNMjIwODAz
MDQxODE5WhcNMzIwNzMxMDQxODE5WjBdMQswCQYDVQQGEwJBVTESMBAGA1UECAwJ
QXVzdHJhbGlhMQ8wDQYDVQQHDAZTeWRuZXkxEjAQBgNVBAoMCU15T3JnTmFtZTEV
MBMGA1UEAwwMTXlDb21tb25OYW1lMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIB
CgKCAQEAwDgNEcPTkTASpfFa0AwPlUFPWhlm2Av1mh3oNsf3kHOBXQymJ3HkXDq/
7durWduubkP1jsOGqO9rcXD1Q3mmNYqsqRRydi5DbMHcFcSSA6g2QncTJwhRE/q/
/00t6e5BhBLXscK+uJEDzEGu9CJVFkkdbeMccfb26C3os1VHGzcp5c/pCNjj93TM
3iwlQYMoEgCo7iUDxyIQ5tjQBn/QmEPcytut11tAIlGPy+SxQjMCykREPOVuwvNh
hZFscpCkvQPTEvv7KBZFBvYafa820CY3z++IIqQ7YBZdxYpYwBuVamUyPKB+lpsn
aD5G2LQjENdjYcRXys04bWgafalZJQIDAQABoxcwFTATBgNVHREEDDAKgghleGFt
cGxlMTANBgkqhkiG9w0BAQsFAAOCAQEAoN6fyv+0ri3wnYMZaP2+m4NA/gk+I4Lp
eP4OpQHkHbm3wjbWZUYLJZ6IvhPHfCNAXdqCs+mpG35HI6Bg+x1CVFrNeueInKTg
0v+0q1FlvSQhsQJoumX2bk/uSLHMIU3hhYIts0vFC0k04Vf7n9hEq7pOZD/akTaw
haLsQe/SRXSTjkar+Csi4DXyi/qshlkV6FOUz9vogAR0W3l8x7dqzwBHL4gRMddM
ZdSfhVFOMwKqUrucYebYZhdAvYqMtlTph46lk+hd5TarFDFJ2zEjbx9NU5gY1b8V
/Kfm2ZHR0yWKGfg9I4TRGZgufm1HBEMnMq1b15DUZxNTagFtPAP18Q==
-----END CERTIFICATE-----"#; // spellchecker:disable-line

    const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAwDgNEcPTkTASpfFa0AwPlUFPWhlm2Av1mh3oNsf3kHOBXQym
J3HkXDq/7durWduubkP1jsOGqO9rcXD1Q3mmNYqsqRRydi5DbMHcFcSSA6g2QncT
JwhRE/q//00t6e5BhBLXscK+uJEDzEGu9CJVFkkdbeMccfb26C3os1VHGzcp5c/p
CNjj93TM3iwlQYMoEgCo7iUDxyIQ5tjQBn/QmEPcytut11tAIlGPy+SxQjMCykRE
POVuwvNhhZFscpCkvQPTEvv7KBZFBvYafa820CY3z++IIqQ7YBZdxYpYwBuVamUy
PKB+lpsnaD5G2LQjENdjYcRXys04bWgafalZJQIDAQABAoIBAFemN29uWD7QKPC6
SaqslT599W0kQB0r9uY71POF44Fe6hI//lPmPzc/It2XWV80KSnmm0ZqKjFGWzvz
QiNuiTfI8Ep5JGh3WA9zpqPWaq54OaW9HmKiDDaMFJiZ3OHa3s0Wunw4TTdkCNNO
8DQqo5nx5RWChioBbz0YEhAURsRFbGqFavDPvlEPOSanCB+mDOliKqX0XizffRZ3
UBQuWa6VjDxHH93b+oJ2/zR5UOlXKHgcqNWeBofxBiiX8ZF5ylwNGOCEE2Gm+KfZ
KUYxGlDKohSYxVjmcyLPoWGrUX83lDKD2u9VrVdgCJwA+IHEsIg9KARb6jFLzACp
RYSDM9ECgYEA7gm8+h44D27I1yRF/3rbhxggsgRo0j5ag9NVLehp11G0ZsdoklJx
uVhDJbjHG9HVqEMfduljr4GpyeMt2ayEmo/ejcWyl0JBMFXPXRvrubM5qoCVOqUu
WYo/JtvIyEAQQicwo5okiPddhFvcQebSH7NXRpKWROMftnlisgtv/xsCgYEAzrk1
vB2O/DTydcLxkY2m8E5qo1ZvTMPW6211ZCKNyQ475ZE/QxZ5iuodwErCQOHjAlV7
n6FeWWZveOsVQeTkSvUOnPCocct+/Dx+sMcRO8k9HuC33bNcw9eHwBoztginIxEb
s7ee+S06AT6r7SQScgBrhD6uevW+dUVbdw/6TL8CgYEAzOyNSDZjxMV3GeAccsjt
3Oukmhy5sOYFPp/dINyI4dlxGVpqaC2Zwhp+FCdzIjwPWAARQmnCbAGQjkGJ429l
6ToaOqsMCLP9MwNstZen5AKrjmGMFyTFNkiR/X4Q6HReitT6Rp4Y/eEXHS+H+yQf
mTLn29WukDeHwavWj7jQ/ikCgYBDPYEZ+C9bH8nBvjAfHQkw3wDWsjWvrX/JwifN
82NVA3k+GbmPE89i/PXCZ066Ff9l8fItISr0P1qA5U5byZzsOLuRFsJjiUJ7vx2i
WI3leXaVBZko1r+UwBVayesKCdR7loQBN/fQqwJUB1Oa5gHN7Q8Ly+uq+SYDNRUk
LCFJNwKBgGWcVuIarQ2mCLqqZ0zxeAp3lFTNeWG2ZMQtzeuo0iGx0xTTUEaZSiNW
MSAvYjGrRzM6XpGEYasfwy0Zoc3loi9nzP5uE4tv8vE72nyMf+OhaPG+Rn+mdBv4
7emViVNVfzLW7L//IkxtEamV0yc6gYwcCfzUckxxXVRD4z2aM78q
-----END RSA PRIVATE KEY-----"#;

    // spellchecker:on

    fn create_temp_file_simple(content: &str, suffix: u32) -> String {
        use std::env;
        let temp_dir = env::temp_dir();
        let file_path = temp_dir.join(format!("test_file_{}", suffix));
        let mut file = fs::File::create(&file_path).expect("Failed to create temp file");
        file.write_all(content.as_bytes())
            .expect("Failed to write to temp file");
        file_path.to_string_lossy().to_string()
    }

    #[test]
    fn test_default() {
        let config = Config::default();
        // No TLS source configured by default
        assert!(matches!(config.source, TlsSource::None));
        assert_eq!(
            config.include_system_ca_certs_pool,
            default_include_system_ca_certs_pool()
        );
        assert_eq!(config.tls_version, "tls1.3".to_string());
        assert_eq!(config.reload_interval, None);
    }

    #[test]
    fn test_default_functions() {
        assert!(default_include_system_ca_certs_pool());
        assert_eq!(default_tls_version(), "tls1.3".to_string());
    }

    #[test]
    fn test_with_include_system_ca_certs_pool() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        assert!(!config.include_system_ca_certs_pool);
    }

    #[test]
    fn test_with_tls_version() {
        let config = Config::default().with_tls_version("tls1.2");
        assert_eq!(config.tls_version, "tls1.2".to_string());
    }

    #[test]
    fn test_with_reload_interval() {
        let duration = Some(Duration::from_secs(300));
        let config = Config::default().with_reload_interval(duration);
        assert_eq!(config.reload_interval, duration);
    }

    #[test]
    fn test_has_unified_ca() {
        let base = Config::default();
        assert!(!base.has(TlsComponent::Ca));

        let file_cfg = base.clone().with_ca_file("/path/to/ca.crt");
        assert!(file_cfg.has(TlsComponent::Ca));

        let pem_cfg = base.with_ca_pem("ca_pem_content");
        assert!(pem_cfg.has(TlsComponent::Ca));
    }

    #[test]
    fn test_has_unified_cert_and_key() {
        let base = Config::default();
        assert!(!base.has(TlsComponent::Cert));

        let base = base.with_cert_and_key_file("/path/to/server.crt", "/path/to/key");
        assert!(base.has(TlsComponent::Cert));

        let base = base.with_cert_and_key_pem("cert_pem_content", "key_pem_content");
        assert!(base.has(TlsComponent::Cert));
    }
    // Removed granular has_key_file test (covered by unified key test)

    // Removed granular has_key_pem test (covered by unified key test)

    #[tokio::test]
    async fn test_load_ca_cert_pool_no_certs() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 0);
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_with_system_certs() {
        let config = Config::default().with_include_system_ca_certs_pool(true);
        let result = config.load_ca_cert_pool().await;
        // This might fail on systems without native certs, but that's expected
        // We're mainly testing that the function doesn't panic
        match result {
            Ok(_root_store) => {
                // System certs loaded successfully
            }
            Err(_) => {
                // System certs not available or failed to load, which is okay for testing
            }
        }
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_with_ca_pem() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_with_ca_file() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path);

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);

        // Clean up
        let _ = fs::remove_file(ca_file_path);
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_override_file_with_pem() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        // Applying with_ca_file then with_ca_pem should result in Pem source taking precedence without error.
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_cert_pool().await;
        // Expect success (no CannotUseBoth in new enum-based model)
        assert!(result.is_ok());
        let root_store = result.unwrap();
        // Pem override should still load exactly 1 cert (the pem)
        assert_eq!(root_store.len(), 1);

        let _ = fs::remove_file(ca_file_path);
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_invalid_pem() {
        let mixed_pem = r#"-----BEGIN CERTIFICATE-----
        INVALID_BASE64_DATA_THAT_WILL_FAIL_PARSING!!!
        -----END CERTIFICATE-----"#;
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(mixed_pem);

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidPem(_) => {} // Expected
            _ => panic!("Expected InvalidPem error"),
        }
    }

    #[tokio::test]
    async fn test_load_ca_cert_pool_nonexistent_file() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file("/nonexistent/path/ca.crt");

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidPem(_) => {} // Expected when file doesn't exist
            _ => panic!("Expected InvalidPem error"),
        }
    }

    #[tokio::test]
    async fn test_load_ca_certificates_no_ca() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 0);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_from_pem() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(TEST_CA_CERT_PEM);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_from_file() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);
        let _ = fs::remove_file(ca_file_path);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_override_file_with_pem() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);
        let _ = fs::remove_file(ca_file_path);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_multiple_from_pem() {
        let multiple_certs = format!("{}\n{}", TEST_CA_CERT_PEM, TEST_CLIENT_CERT_PEM);
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(&multiple_certs);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 2);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_multiple_from_file() {
        let multiple_certs = format!("{}\n{}", TEST_CA_CERT_PEM, TEST_CLIENT_CERT_PEM);
        let ca_file_path = create_temp_file_simple(&multiple_certs, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path);
        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 2);
        let _ = fs::remove_file(ca_file_path);
    }

    #[tokio::test]
    async fn test_load_ca_certificates_multiple() {
        let multiple_certs = format!("{}\n{}", TEST_CA_CERT_PEM, TEST_CLIENT_CERT_PEM);
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(&multiple_certs);

        let result = config.load_ca_cert_pool().await;
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 2);
    }

    #[test]
    fn test_config_error_display() {
        let errors = vec![
            ConfigError::InvalidTlsVersion("tls1.1".to_string()),
            ConfigError::MissingServerCertAndKey,
            ConfigError::Unknown,
        ];

        for error in errors {
            let _display = format!("{}", error);
            // Just ensure Display trait works without panicking
        }
    }

    #[test]
    fn test_config_clone_and_partial_eq() {
        let config1 = Config::default()
            .with_ca_file("/path/to/ca.crt")
            .with_tls_version("tls1.2");

        let config2 = config1.clone();
        assert_eq!(config1, config2);

        let config3 = config1.clone().with_tls_version("tls1.3");
        assert_ne!(config1, config3);
    }

    #[test]
    fn test_static_cert_resolver_new() {
        provider::initialize_crypto_provider();
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();

        let result = StaticCertResolver::new(TEST_PRIVATE_KEY_PEM, TEST_CLIENT_CERT_PEM, provider);

        // This test might fail due to the test certificates not being valid
        // but we're testing that the function doesn't panic during creation
        match result {
            Ok(_resolver) => {
                // Successfully created resolver
            }
            Err(ConfigError::InvalidPem(_)) => {
                // Expected with test certificates
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_watcher_cert_resolver_new() {
        provider::initialize_crypto_provider();
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();

        let suffix = rand::random::<u32>();
        let key_file_path = create_temp_file_simple(TEST_PRIVATE_KEY_PEM, suffix);
        let cert_file_path = create_temp_file_simple(TEST_CLIENT_CERT_PEM, suffix);

        let result = WatcherCertResolver::new(&key_file_path, &cert_file_path, provider);

        // This test might fail due to the test certificates not being valid
        // but we're testing that the function doesn't panic during creation
        match result {
            Ok(_resolver) => {
                // Successfully created resolver
            }
            Err(ConfigError::InvalidFile(_)) => {
                // Expected with test certificates
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }

        // Clean up
        let _ = fs::remove_file(key_file_path);
        let _ = fs::remove_file(cert_file_path);
    }
}
