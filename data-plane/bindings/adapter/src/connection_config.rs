// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/// TLS configuration for server/client
#[derive(uniffi::Record)]
pub struct TlsConfig {
    /// Disable TLS entirely (plain text connection)
    pub insecure: bool,

    /// Skip server certificate verification (client only, enables TLS but doesn't verify certs)
    /// WARNING: Only use for testing - insecure in production!
    pub insecure_skip_verify: Option<bool>,

    /// Path to certificate file (PEM format)
    pub cert_file: Option<String>,

    /// Path to private key file (PEM format)
    pub key_file: Option<String>,

    /// Path to CA certificate file (PEM format) for verifying peer certificates
    pub ca_file: Option<String>,

    /// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
    pub tls_version: Option<String>,

    /// Include system CA certificates pool (default: false)
    pub include_system_ca_certs_pool: Option<bool>,
}

/// Server configuration for running a SLIM server
#[derive(uniffi::Record)]
pub struct ServerConfig {
    pub endpoint: String,
    pub tls: TlsConfig,
}

/// Client configuration for connecting to a SLIM server
#[derive(uniffi::Record)]
pub struct ClientConfig {
    pub endpoint: String,
    pub tls: TlsConfig,
}
