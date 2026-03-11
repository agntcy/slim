// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod client;
#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod common;
#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod errors;
#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod provider;
#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod root_store_builder;
#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub mod server;

#[cfg(any(feature = "grpc", feature = "websocket-native"))]
pub use root_store_builder::RootStoreBuilder;

#[cfg(not(any(feature = "grpc", feature = "websocket-native")))]
pub mod errors {
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum ConfigError {
        #[error("TLS is unavailable for this build configuration")]
        Unsupported,
    }
}

#[cfg(not(any(feature = "grpc", feature = "websocket-native")))]
pub mod client {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use crate::component::configuration::Configuration;
    use crate::tls::errors::ConfigError;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
    pub struct TlsClientConfig {
        #[serde(default = "default_insecure")]
        pub insecure: bool,
        #[serde(default)]
        pub insecure_skip_verify: bool,
    }

    impl Default for TlsClientConfig {
        fn default() -> Self {
            Self {
                insecure: default_insecure(),
                insecure_skip_verify: false,
            }
        }
    }

    fn default_insecure() -> bool {
        false
    }

    impl TlsClientConfig {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn insecure() -> Self {
            Self {
                insecure: true,
                ..Self::default()
            }
        }

        pub fn with_insecure_skip_verify(self, insecure_skip_verify: bool) -> Self {
            Self {
                insecure_skip_verify,
                ..self
            }
        }

        pub fn with_insecure(self, insecure: bool) -> Self {
            Self { insecure, ..self }
        }

        pub fn with_ca_file(self, _ca_file: &str) -> Self {
            self
        }

        pub fn with_ca_pem(self, _ca_pem: &str) -> Self {
            self
        }

        pub fn with_include_system_ca_certs_pool(self, _include: bool) -> Self {
            self
        }

        pub fn with_cert_and_key_file(self, _cert_file: &str, _key_file: &str) -> Self {
            self
        }

        pub fn with_cert_and_key_pem(self, _cert_pem: &str, _key_pem: &str) -> Self {
            self
        }

        pub fn with_tls_version(self, _tls_version: &str) -> Self {
            self
        }

        pub fn with_reload_interval(self, _reload_interval: Option<std::time::Duration>) -> Self {
            self
        }
    }

    impl std::fmt::Display for TlsClientConfig {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl Configuration for TlsClientConfig {
        type Error = ConfigError;

        fn validate(&self) -> Result<(), Self::Error> {
            Ok(())
        }
    }
}

#[cfg(not(any(feature = "grpc", feature = "websocket-native")))]
pub mod server {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use crate::component::configuration::Configuration;
    use crate::tls::errors::ConfigError;

    #[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
    pub struct TlsServerConfig {
        #[serde(default = "default_insecure")]
        pub insecure: bool,
        #[serde(default = "default_reload_client_ca_file")]
        pub reload_client_ca_file: bool,
    }

    impl Default for TlsServerConfig {
        fn default() -> Self {
            Self {
                insecure: default_insecure(),
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

    impl TlsServerConfig {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn insecure() -> Self {
            Self {
                insecure: true,
                ..Self::default()
            }
        }

        pub fn with_insecure(self, insecure: bool) -> Self {
            Self { insecure, ..self }
        }

        pub fn with_reload_client_ca_file(self, reload_client_ca_file: bool) -> Self {
            Self {
                reload_client_ca_file,
                ..self
            }
        }

        pub fn with_ca_file(self, _ca_file: &str) -> Self {
            self
        }

        pub fn with_ca_pem(self, _ca_pem: &str) -> Self {
            self
        }

        pub fn with_include_system_ca_certs_pool(self, _include: bool) -> Self {
            self
        }

        pub fn with_cert_and_key_file(self, _cert_path: &str, _key_path: &str) -> Self {
            self
        }

        pub fn with_cert_and_key_pem(self, _cert_pem: &str, _key_pem: &str) -> Self {
            self
        }

        pub fn with_tls_version(self, _tls_version: &str) -> Self {
            self
        }

        pub fn with_reload_interval(self, _reload_interval: Option<std::time::Duration>) -> Self {
            self
        }
    }

    impl std::fmt::Display for TlsServerConfig {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl Configuration for TlsServerConfig {
        type Error = ConfigError;

        fn validate(&self) -> Result<(), Self::Error> {
            Ok(())
        }
    }
}
