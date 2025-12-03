// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! TLS configuration tests asserting structured `ConfigError` variants instead
//! of performing brittle string matching.

use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::common::{CaSource, Config, RustlsConfigLoader, TlsSource};
use slim_config::tls::errors::ConfigError;
use slim_config::tls::server::TlsServerConfig;

static TEST_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/tls");

#[test]
fn test_new_config() {
    let expected_config = Config {
        ..Default::default()
    };
    let config = Config::default();
    assert_eq!(config, expected_config);
}

#[test]
fn test_new_client_config() {
    let expected_client_config = TlsClientConfig {
        ..Default::default()
    };
    let client_config = TlsClientConfig::default();
    assert_eq!(client_config, expected_client_config);
}

#[test]
fn test_new_server_config() {
    let expected_server_config = TlsServerConfig {
        ..Default::default()
    };
    let server_config = TlsServerConfig::default();
    assert_eq!(server_config, expected_server_config);
}

/// Expected high–level error classification for test cases.
///
/// We avoid matching on internal strings and instead use the enum shape.
#[derive(Debug, Clone, Copy)]
enum ExpectedErrorVariant {
    /// Expect success (`Ok(_)`)
    Ok,
    /// Any error is acceptable (use when the precise variant may differ per platform)
    AnyError,
    InvalidTlsVersion,
    InvalidPem,
    MissingServerCertAndKey,
}

impl ExpectedErrorVariant {
    fn assert(&self, result: &Result<Option<impl std::fmt::Debug>, ConfigError>) {
        match self {
            ExpectedErrorVariant::Ok => {
                assert!(
                    result.is_ok(),
                    "Expected Ok but got Err: {:?}",
                    result.as_ref().err()
                );
            }
            ExpectedErrorVariant::AnyError => {
                assert!(result.is_err(), "Expected an error but got Ok");
            }
            ExpectedErrorVariant::InvalidTlsVersion => {
                let err = result
                    .as_ref()
                    .expect_err("Expected InvalidTlsVersion error");
                assert!(
                    matches!(err, ConfigError::InvalidTlsVersion(_)),
                    "Expected InvalidTlsVersion, got: {err:?}"
                );
            }
            ExpectedErrorVariant::InvalidPem => {
                let err = result.as_ref().expect_err("Expected InvalidPem error");
                assert!(
                    matches!(err, ConfigError::InvalidPem(_)),
                    "Expected InvalidPem, got: {err:?}"
                );
            }
            ExpectedErrorVariant::MissingServerCertAndKey => {
                let err = result
                    .as_ref()
                    .expect_err("Expected MissingServerCertAndKey error");
                assert!(
                    matches!(err, ConfigError::MissingServerCertAndKey),
                    "Expected MissingServerCertAndKey, got: {err:?}"
                );
            }
        }
    }
}

async fn assert_load_rustls<T: std::fmt::Debug>(
    name: &str,
    loader: &dyn RustlsConfigLoader<T>,
    expected: ExpectedErrorVariant,
    print_error: bool,
) {
    let result = loader.load_rustls_config().await;
    if print_error && result.is_err() {
        println!("Test {name} produced error: {:?}", result.as_ref().err());
    }
    expected.assert(&result);
}

#[tokio::test]
async fn test_load_rustls_client() {
    // Crypto provider setup
    slim_config::tls::provider::initialize_crypto_provider();

    struct ClientCase {
        name: &'static str,
        build: Box<dyn Fn() -> TlsClientConfig>,
        expected: ExpectedErrorVariant,
        print_error: bool,
    }

    let cases: Vec<ClientCase> = vec![
        ClientCase {
            name: "valid-ca-1",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-1.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "valid-ca-2",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-2.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        // File not found (variant may differ depending on underlying library / OS)
        ClientCase {
            name: "ca-file-not-found",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::AnyError,
            print_error: false,
        },
        ClientCase {
            name: "client-certificate-file",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "client-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "client-1.key"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "tls-version-valid",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    tls_version: "tls1.2".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "tls-version-invalid",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    tls_version: "tls1.4".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::InvalidTlsVersion,
            print_error: false,
        },
        ClientCase {
            name: "ca-pem-valid",
            build: Box::new(|| {
                let ca_pem =
                    std::fs::read_to_string(format!("{}/{}", TEST_PATH, "ca-2.crt")).expect("read");
                TlsClientConfig {
                    config: Config {
                        ca_source: CaSource::Pem { data: ca_pem },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "ca-pem-invalid",
            build: Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::Pem {
                        data: "-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----"
                            .to_string(),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::InvalidPem,
            print_error: false,
        },
        ClientCase {
            name: "client-certificate-pem-valid",
            build: Box::new(|| {
                let cert_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "client-1.crt"))
                    .expect("read cert");
                let key_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "client-1.key"))
                    .expect("read key");
                TlsClientConfig {
                    config: Config {
                        source: TlsSource::Pem {
                            cert: cert_pem,
                            key: key_pem,
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "client-certificate-pem-invalid",
            build: Box::new(|| {
                let cert_pem =
                    "-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----".to_string();
                let key_pem =
                    "-----BEGIN PRIVATE KEY-----\nwrong\n-----END PRIVATE KEY-----".to_string();
                TlsClientConfig {
                    config: Config {
                        source: TlsSource::Pem {
                            cert: cert_pem,
                            key: key_pem,
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
            expected: ExpectedErrorVariant::InvalidPem,
            print_error: false,
        },
        ClientCase {
            name: "insecure-no-ca",
            build: Box::new(|| TlsClientConfig {
                insecure: true,
                ..Default::default()
            }),
            // Insecure path yields Ok(None)
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ClientCase {
            name: "insecure-skip-verify",
            build: Box::new(|| TlsClientConfig {
                insecure_skip_verify: true,
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
    ];

    for case in cases {
        let cfg = (case.build)();
        assert_load_rustls(case.name, &cfg, case.expected, case.print_error).await;
    }
}

#[tokio::test]
async fn test_load_rustls_server() {
    // Crypto provider setup
    slim_config::tls::provider::initialize_crypto_provider();

    struct ServerCase {
        name: &'static str,
        build: Box<dyn Fn() -> TlsServerConfig>,
        expected: ExpectedErrorVariant,
        print_error: bool,
    }

    let cases: Vec<ServerCase> = vec![
        ServerCase {
            name: "no-certificate-file",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::None,
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::MissingServerCertAndKey,
            print_error: true,
        },
        ServerCase {
            name: "no-key-file",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::None,
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::MissingServerCertAndKey,
            print_error: false,
        },
        ServerCase {
            name: "server-certificate-file",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ServerCase {
            name: "tls-version-valid",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    tls_version: "tls1.2".to_string(),
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ServerCase {
            name: "tls-version-invalid",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    tls_version: "tls1.4".to_string(),
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::InvalidTlsVersion,
            print_error: false,
        },
        ServerCase {
            name: "client-ca-file-valid",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                client_ca: CaSource::File {
                    path: format!("{}/{}", TEST_PATH, "ca-1.crt"),
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        // File may be missing (variant can differ); allow any error.
        ServerCase {
            name: "client-ca-file-not-found",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                client_ca: CaSource::File {
                    path: format!("{}/{}", TEST_PATH, "ca1.crt"),
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::AnyError,
            print_error: false,
        },
        ServerCase {
            name: "client-ca-pem-valid",
            build: Box::new(|| {
                let ca_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "ca-2.crt"))
                    .expect("read ca");
                TlsServerConfig {
                    config: Config {
                        source: TlsSource::File {
                            cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                            key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        },
                        ..Default::default()
                    },
                    client_ca: CaSource::Pem { data: ca_pem },
                    ..Default::default()
                }
            }),
            expected: ExpectedErrorVariant::Ok,
            print_error: false,
        },
        ServerCase {
            name: "client-ca-pem-invalid",
            build: Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                    },
                    ..Default::default()
                },
                client_ca: CaSource::Pem {
                    data: "-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----"
                        .to_string(),
                },
                ..Default::default()
            }),
            expected: ExpectedErrorVariant::InvalidPem,
            print_error: false,
        },
    ];

    for case in cases {
        let cfg = (case.build)();
        assert_load_rustls(case.name, &cfg, case.expected, case.print_error).await;
    }
}
