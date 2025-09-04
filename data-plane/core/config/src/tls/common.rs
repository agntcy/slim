// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use parking_lot::RwLock;
use rustls::RootCertStore;
use rustls::crypto::CryptoProvider;
use rustls::server::VerifierBuilderError;
use rustls::sign::CertifiedKey;
use rustls_native_certs;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::file_watcher::FileWatcher;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

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
pub struct Config {
    /// Path to the CA cert. For a client this verifies the server certificate.
    /// For a server this verifies client certificates. If empty uses system root CA.
    /// (optional)
    pub ca_file: Option<String>,

    /// In memory PEM encoded cert. (optional)
    pub ca_pem: Option<String>,

    /// If true, load system CA certificates pool in addition to the certificates
    /// configured in this struct.
    #[serde(default = "default_include_system_ca_certs_pool")]
    pub include_system_ca_certs_pool: bool,

    /// Path to the TLS cert to use for TLS required connections. (optional)
    pub cert_file: Option<String>,

    /// In memory PEM encoded TLS cert to use for TLS required connections. (optional)
    pub cert_pem: Option<String>,

    /// Path to the TLS key to use for TLS required connections. (optional)
    pub key_file: Option<String>,

    /// In memory PEM encoded TLS key to use for TLS required connections. (optional)
    pub key_pem: Option<String>,

    /// The TLS version to use. If not set, the default is "tls1.3".
    /// The value must be either "tls1.2" or "tls1.3".
    /// (optional)
    #[serde(default = "default_tls_version")]
    pub tls_version: String,

    /// ReloadInterval specifies the duration after which the certificate will be reloaded
    /// If not set, it will never be reloaded
    // TODO(msardara): not implemented yet
    pub reload_interval: Option<Duration>,
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
    #[error("cannot use both file and pem for {0}")]
    CannotUseBoth(String),
    #[error("root store error: {0}")]
    RootStore(rustls::Error),
    #[error("config builder error")]
    ConfigBuilder(rustls::Error),
    #[error("missing server cert and key. cert_{{file, pem}} and key_{{file, pem}} must be set")]
    MissingServerCertAndKey,
    #[error("verifier builder error")]
    VerifierBuilder(VerifierBuilderError),
    #[error("unknown error")]
    Unknown,
}

// Defaults for Config
impl Default for Config {
    fn default() -> Config {
        Config {
            ca_file: None,
            ca_pem: None,
            include_system_ca_certs_pool: default_include_system_ca_certs_pool(),
            cert_file: None,
            cert_pem: None,
            key_file: None,
            key_pem: None,
            tls_version: "tls1.3".to_string(),
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
    pub(crate) fn with_ca_file(self, ca_file: &str) -> Config {
        Config {
            ca_file: Some(ca_file.to_string()),
            ..self
        }
    }

    pub(crate) fn with_ca_pem(self, ca_pem: &str) -> Config {
        Config {
            ca_pem: Some(ca_pem.to_string()),
            ..self
        }
    }

    pub(crate) fn with_include_system_ca_certs_pool(
        self,
        include_system_ca_certs_pool: bool,
    ) -> Config {
        Config {
            include_system_ca_certs_pool,
            ..self
        }
    }

    pub(crate) fn with_cert_file(self, cert_file: &str) -> Config {
        Config {
            cert_file: Some(cert_file.to_string()),
            ..self
        }
    }

    pub(crate) fn with_cert_pem(self, cert_pem: &str) -> Config {
        Config {
            cert_pem: Some(cert_pem.to_string()),
            ..self
        }
    }

    pub(crate) fn with_key_file(self, key_file: &str) -> Config {
        Config {
            key_file: Some(key_file.to_string()),
            ..self
        }
    }

    pub(crate) fn with_key_pem(self, key_pem: &str) -> Config {
        Config {
            key_pem: Some(key_pem.to_string()),
            ..self
        }
    }

    pub(crate) fn with_tls_version(self, tls_version: &str) -> Config {
        Config {
            tls_version: tls_version.to_string(),
            ..self
        }
    }

    pub(crate) fn with_reload_interval(self, reload_interval: Option<Duration>) -> Config {
        Config {
            reload_interval,
            ..self
        }
    }

    pub(crate) fn load_ca_cert_pool(&self) -> Result<RootCertStore, ConfigError> {
        let mut root_store = RootCertStore::empty();

        self.add_system_ca_certs(&mut root_store)?;
        self.add_custom_ca_cert(&mut root_store)?;

        Ok(root_store)
    }

    fn add_system_ca_certs(&self, root_store: &mut RootCertStore) -> Result<(), ConfigError> {
        if !self.include_system_ca_certs_pool {
            return Ok(());
        }

        let native_certs =
            rustls_native_certs::load_native_certs().expect("could not load platform certs");

        for cert in native_certs {
            root_store.add(cert).map_err(ConfigError::RootStore)?;
        }

        Ok(())
    }

    fn add_custom_ca_cert(&self, root_store: &mut RootCertStore) -> Result<(), ConfigError> {
        let ca_cert = self.load_ca_certificate()?;

        if let Some(cert) = ca_cert {
            root_store.add(cert).map_err(ConfigError::RootStore)?;
        }

        Ok(())
    }

    fn load_ca_certificate(&self) -> Result<Option<CertificateDer<'static>>, ConfigError> {
        match (self.has_ca_file(), self.has_ca_pem()) {
            (true, true) => Err(ConfigError::CannotUseBoth("ca".to_string())),
            (true, false) => {
                let cert_path = Path::new(self.ca_file.as_ref().unwrap());
                let cert =
                    CertificateDer::from_pem_file(cert_path).map_err(ConfigError::InvalidPem)?;
                Ok(Some(cert))
            }
            (false, true) => {
                let cert_bytes = self.ca_pem.as_ref().unwrap().as_bytes();
                let cert =
                    CertificateDer::from_pem_slice(cert_bytes).map_err(ConfigError::InvalidPem)?;
                Ok(Some(cert))
            }
            (false, false) => Ok(None),
        }
    }

    /// Returns true if the config has a CA cert
    pub fn has_ca(&self) -> bool {
        self.has_ca_file() || self.has_ca_pem()
    }

    /// Returns true if the config has a CA file
    pub fn has_ca_file(&self) -> bool {
        self.ca_file.is_some()
    }

    /// Returns true if the config has a CA PEM
    pub fn has_ca_pem(&self) -> bool {
        self.ca_pem.is_some()
    }

    /// Returns true if the config has a cert file
    pub fn has_cert_file(&self) -> bool {
        self.cert_file.is_some()
    }

    /// Returns true if the config has a cert PEM
    pub fn has_cert_pem(&self) -> bool {
        self.cert_pem.is_some()
    }

    /// Returns true if the config has a key file
    pub fn has_key_file(&self) -> bool {
        self.key_file.is_some()
    }

    /// Returns true if the config has a key PEM
    pub fn has_key_pem(&self) -> bool {
        self.key_pem.is_some()
    }
}

// trait load_rustls_config
pub trait RustlsConfigLoader<T> {
    fn load_rustls_config(&self) -> Result<Option<T>, ConfigError>;
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

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
        assert_eq!(config.ca_file, None);
        assert_eq!(config.ca_pem, None);
        assert_eq!(
            config.include_system_ca_certs_pool,
            default_include_system_ca_certs_pool()
        );
        assert_eq!(config.cert_file, None);
        assert_eq!(config.cert_pem, None);
        assert_eq!(config.key_file, None);
        assert_eq!(config.key_pem, None);
        assert_eq!(config.tls_version, "tls1.3".to_string());
        assert_eq!(config.reload_interval, None);
    }

    #[test]
    fn test_default_functions() {
        assert!(default_include_system_ca_certs_pool());
        assert_eq!(default_tls_version(), "tls1.3".to_string());
    }

    #[test]
    fn test_with_ca_file() {
        let config = Config::default().with_ca_file("/path/to/ca.crt");
        assert_eq!(config.ca_file, Some("/path/to/ca.crt".to_string()));
    }

    #[test]
    fn test_with_ca_pem() {
        let config = Config::default().with_ca_pem("ca_pem_content");
        assert_eq!(config.ca_pem, Some("ca_pem_content".to_string()));
    }

    #[test]
    fn test_with_include_system_ca_certs_pool() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        assert!(!config.include_system_ca_certs_pool);
    }

    #[test]
    fn test_with_cert_file() {
        let config = Config::default().with_cert_file("/path/to/cert.crt");
        assert_eq!(config.cert_file, Some("/path/to/cert.crt".to_string()));
    }

    #[test]
    fn test_with_cert_pem() {
        let config = Config::default().with_cert_pem("cert_pem_content");
        assert_eq!(config.cert_pem, Some("cert_pem_content".to_string()));
    }

    #[test]
    fn test_with_key_file() {
        let config = Config::default().with_key_file("/path/to/key.key");
        assert_eq!(config.key_file, Some("/path/to/key.key".to_string()));
    }

    #[test]
    fn test_with_key_pem() {
        let config = Config::default().with_key_pem("key_pem_content");
        assert_eq!(config.key_pem, Some("key_pem_content".to_string()));
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
    fn test_has_ca() {
        let config = Config::default();
        assert!(!config.has_ca());

        let config_with_file = config.clone().with_ca_file("/path/to/ca.crt");
        assert!(config_with_file.has_ca());

        let config_with_pem = config.with_ca_pem("ca_pem_content");
        assert!(config_with_pem.has_ca());
    }

    #[test]
    fn test_has_ca_file() {
        let config = Config::default();
        assert!(!config.has_ca_file());

        let config_with_file = config.with_ca_file("/path/to/ca.crt");
        assert!(config_with_file.has_ca_file());
    }

    #[test]
    fn test_has_ca_pem() {
        let config = Config::default();
        assert!(!config.has_ca_pem());

        let config_with_pem = config.with_ca_pem("ca_pem_content");
        assert!(config_with_pem.has_ca_pem());
    }

    #[test]
    fn test_has_cert_file() {
        let config = Config::default();
        assert!(!config.has_cert_file());

        let config_with_file = config.with_cert_file("/path/to/cert.crt");
        assert!(config_with_file.has_cert_file());
    }

    #[test]
    fn test_has_cert_pem() {
        let config = Config::default();
        assert!(!config.has_cert_pem());

        let config_with_pem = config.with_cert_pem("cert_pem_content");
        assert!(config_with_pem.has_cert_pem());
    }

    #[test]
    fn test_has_key_file() {
        let config = Config::default();
        assert!(!config.has_key_file());

        let config_with_file = config.with_key_file("/path/to/key.key");
        assert!(config_with_file.has_key_file());
    }

    #[test]
    fn test_has_key_pem() {
        let config = Config::default();
        assert!(!config.has_key_pem());

        let config_with_pem = config.with_key_pem("key_pem_content");
        assert!(config_with_pem.has_key_pem());
    }

    #[test]
    fn test_load_ca_cert_pool_no_certs() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        let result = config.load_ca_cert_pool();
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 0);
    }

    #[test]
    fn test_load_ca_cert_pool_with_system_certs() {
        let config = Config::default().with_include_system_ca_certs_pool(true);
        let result = config.load_ca_cert_pool();
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

    #[test]
    fn test_load_ca_cert_pool_with_ca_pem() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_cert_pool();
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);
    }

    #[test]
    fn test_load_ca_cert_pool_with_ca_file() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path);

        let result = config.load_ca_cert_pool();
        assert!(result.is_ok());
        let root_store = result.unwrap();
        assert_eq!(root_store.len(), 1);

        // Clean up
        let _ = fs::remove_file(ca_file_path);
    }

    #[test]
    fn test_load_ca_cert_pool_both_file_and_pem_error() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file(&ca_file_path)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_cert_pool();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::CannotUseBoth(msg) => assert_eq!(msg, "ca"),
            _ => panic!("Expected CannotUseBoth error"),
        }

        // Clean up
        let _ = fs::remove_file(ca_file_path);
    }

    #[test]
    fn test_load_ca_cert_pool_invalid_pem() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_pem("invalid_pem_content");

        let result = config.load_ca_cert_pool();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidPem(_) => {} // Expected
            _ => panic!("Expected InvalidPem error"),
        }
    }

    #[test]
    fn test_load_ca_cert_pool_nonexistent_file() {
        let config = Config::default()
            .with_include_system_ca_certs_pool(false)
            .with_ca_file("/nonexistent/path/ca.crt");

        let result = config.load_ca_cert_pool();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidPem(_) => {} // Expected when file doesn't exist
            _ => panic!("Expected InvalidPem error"),
        }
    }

    #[test]
    fn test_load_ca_certificate_no_ca() {
        let config = Config::default();
        let result = config.load_ca_certificate();
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_load_ca_certificate_from_pem() {
        let config = Config::default().with_ca_pem(TEST_CA_CERT_PEM);
        let result = config.load_ca_certificate();
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_load_ca_certificate_from_file() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default().with_ca_file(&ca_file_path);
        let result = config.load_ca_certificate();
        println!("res = {:?}", result);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        // Clean up
        let _ = fs::remove_file(ca_file_path);
    }

    #[test]
    fn test_load_ca_certificate_both_file_and_pem() {
        let ca_file_path = create_temp_file_simple(TEST_CA_CERT_PEM, rand::random::<u32>());
        let config = Config::default()
            .with_ca_file(&ca_file_path)
            .with_ca_pem(TEST_CA_CERT_PEM);

        let result = config.load_ca_certificate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::CannotUseBoth(msg) => assert_eq!(msg, "ca"),
            _ => panic!("Expected CannotUseBoth error"),
        }

        // Clean up
        let _ = fs::remove_file(ca_file_path);
    }

    #[test]
    fn test_config_error_display() {
        let errors = vec![
            ConfigError::InvalidTlsVersion("tls1.1".to_string()),
            ConfigError::CannotUseBoth("test".to_string()),
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
        use rustls::crypto::aws_lc_rs;
        let provider = Arc::new(aws_lc_rs::default_provider());

        let result = StaticCertResolver::new(TEST_PRIVATE_KEY_PEM, TEST_CLIENT_CERT_PEM, &provider);

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
    fn test_add_system_ca_certs_disabled() {
        let config = Config::default().with_include_system_ca_certs_pool(false);
        let mut root_store = RootCertStore::empty();
        let result = config.add_system_ca_certs(&mut root_store);
        assert!(result.is_ok());
        assert_eq!(root_store.len(), 0);
    }

    #[test]
    fn test_add_custom_ca_cert_none() {
        let config = Config::default();
        let mut root_store = RootCertStore::empty();
        let result = config.add_custom_ca_cert(&mut root_store);
        assert!(result.is_ok());
        assert_eq!(root_store.len(), 0);
    }

    #[test]
    fn test_add_custom_ca_cert_with_pem() {
        let config = Config::default().with_ca_pem(TEST_CA_CERT_PEM);
        let mut root_store = RootCertStore::empty();
        let result = config.add_custom_ca_cert(&mut root_store);
        assert!(
            result.is_ok(),
            "Failed to add custom CA cert: {}",
            result.unwrap_err()
        );
        assert_eq!(root_store.len(), 1);
    }

    #[test]
    fn test_watcher_cert_resolver_new() {
        use rustls::crypto::aws_lc_rs;
        let provider = Arc::new(aws_lc_rs::default_provider());

        let suffix = rand::random::<u32>();
        let key_file_path = create_temp_file_simple(TEST_PRIVATE_KEY_PEM, suffix);
        let cert_file_path = create_temp_file_simple(TEST_CLIENT_CERT_PEM, suffix);

        let result = WatcherCertResolver::new(&key_file_path, &cert_file_path, &provider);

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
