// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use tracing::info;

use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::common::{CaSource, Config, RustlsConfigLoader, TlsSource};
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

// debug trait
#[derive(Debug)]
enum ErrorMessage {
    Empty,
    Is(String),
}

impl Default for ErrorMessage {
    fn default() -> Self {
        ErrorMessage::Is(String::from("world"))
    }
}

// server config tests
async fn test_load_rustls_config<T>(
    test_name: &str,
    config: &dyn RustlsConfigLoader<T>,
    error_expected: &bool,
    error_message: &ErrorMessage,
    print_error: &bool,
) {
    info!("Running test {}", test_name);
    println!("Running test {}", test_name);

    // Try to create a tls config from the server config
    let result = config.load_rustls_config().await;
    match error_expected {
        true => assert!(result.is_err()),
        false => assert!(result.is_ok()),
    }

    if *print_error {
        info!(
            "Error in test {}: {:?}",
            test_name,
            result.as_ref().err().unwrap().to_string()
        );
    }

    match error_message {
        ErrorMessage::Empty => (),
        ErrorMessage::Is(message) => {
            if *error_expected {
                println!(
                    "Error in test {}: {:?}",
                    test_name,
                    result.as_ref().err().unwrap().to_string()
                );
                assert!(
                    result.as_ref().err().unwrap().to_string().contains(message),
                    "{}",
                    test_name
                );
            }
        }
    }
}

#[tokio::test]
async fn test_load_rustls_client() {
    // Crypto provider setup
    slim_config::tls::provider::initialize_crypto_provider();

    let tests = [
        (
            "test-valid-ca-1",
            Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-1.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-valid-ca-2",
            Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-2.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-ca-file-not-found",
            Box::new(|| TlsClientConfig {
                config: Config {
                    ca_source: CaSource::File {
                        path: format!("{}/{}", TEST_PATH, "ca-.crt"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            true,
            ErrorMessage::Is(String::from("No such file or directory")),
            false,
        ),
        (
            "test-client-certificate-file",
            Box::new(|| TlsClientConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "client-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "client-1.key"),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-tls-version",
            Box::new(|| TlsClientConfig {
                config: Config {
                    tls_version: String::from("tls1.2"),
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-tls-version-invalid",
            Box::new(|| TlsClientConfig {
                config: Config {
                    tls_version: String::from("tls1.4"),
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            true,
            ErrorMessage::Is(String::from("invalid tls version")),
            false,
        ),
        (
            "test-ca-pem",
            Box::new(|| {
                // read ca pem from file and set it as ca pem
                let ca_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "ca-2.crt"))
                    .expect("Unable to read file");

                TlsClientConfig {
                    config: Config {
                        ca_source: CaSource::Pem { data: ca_pem },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-ca-pem-invalid",
            Box::new(|| {
                // set wrong PEM
                TlsClientConfig {
                    config: Config {
                        ca_source: CaSource::Pem {
                            data: "-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----"
                                .to_string(),
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }) as Box<dyn Fn() -> TlsClientConfig>,
            true,
            ErrorMessage::Is(String::from("invalid pem format")),
            false,
        ),
        (
            "test-client-certificate-pem",
            Box::new(|| {
                // read ca pem from file and set it as ca pem
                let cert_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "client-1.crt"))
                    .expect("Unable to read cert file");

                let key_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "client-1.key"))
                    .expect("Unable to read key file");

                TlsClientConfig {
                    config: Config {
                        source: TlsSource::Pem {
                            cert: cert_pem,
                            key: key_pem,
                            // removed legacy ca field and Option wrapping
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-client-certificate-pem-wrong",
            Box::new(|| {
                // read ca pem from file and set it as ca pem
                let cert_pem =
                    String::from("-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----");
                let key_pem = String::from("-----BEGIN PRIVATE-----\nwrong\n-----END PRIVATE-----");

                TlsClientConfig {
                    config: Config {
                        source: TlsSource::Pem {
                            cert: cert_pem,
                            key: key_pem,
                            // removed legacy ca field and Option wrapping
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }) as Box<dyn Fn() -> TlsClientConfig>,
            true,
            ErrorMessage::Is(String::from("invalid pem format")),
            false,
        ),
        (
            "test-insecure",
            Box::new(|| TlsClientConfig {
                config: Config {
                    ..Default::default()
                },
                insecure: true,
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-insecure-skip-validation",
            Box::new(|| TlsClientConfig {
                config: Config {
                    ..Default::default()
                },
                insecure_skip_verify: true,
                ..Default::default()
            }) as Box<dyn Fn() -> TlsClientConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
    ];

    for (test_name, client_config, error_expected, error_message, print_error) in tests.iter() {
        let config = (**client_config)();

        test_load_rustls_config(
            test_name,
            &config,
            error_expected,
            error_message,
            print_error,
        )
        .await;
    }
}

#[tokio::test]
async fn test_load_rustls_server() {
    // Crypto provider setup
    slim_config::tls::provider::initialize_crypto_provider();

    let tests = [
        (
            "test-no-certificate-file",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::None,
                    // legacy cert_source fields removed
                    // using None variant to trigger MissingServerCertAndKey error
                    //
                    //
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            true,
            ErrorMessage::Is(String::from("missing server cert and key")),
            true,
        ),
        (
            "test-no-key-file",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::None,
                    // legacy partial File source removed
                    // using None variant to trigger MissingServerCertAndKey error
                    //
                    //
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            true,
            ErrorMessage::Is(String::from("missing server cert and key")),
            false,
        ),
        (
            "test-server-certificate-file",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-tls-version",
            Box::new(|| TlsServerConfig {
                config: Config {
                    tls_version: String::from("tls1.2"),
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-tls-version-invalid",
            Box::new(|| TlsServerConfig {
                config: Config {
                    tls_version: String::from("tls1.4"),
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            true,
            ErrorMessage::Is(String::from("invalid tls version")),
            false,
        ),
        (
            "test-client-ca-file",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                client_ca: CaSource::File {
                    path: format!("{}/{}", TEST_PATH, "ca-1.crt"),
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-client-ca-file-not-found",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                client_ca: CaSource::File {
                    path: format!("{}/{}", TEST_PATH, "ca1.crt"),
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            true,
            ErrorMessage::Is(String::from("No such file or directory")),
            false,
        ),
        (
            "test-client-ca-pem",
            Box::new(|| {
                // read ca pem from file and set it as ca pem
                let ca_pem = std::fs::read_to_string(format!("{}/{}", TEST_PATH, "ca-2.crt"))
                    .expect("Unable to read file");

                TlsServerConfig {
                    config: Config {
                        source: TlsSource::File {
                            cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                            key: format!("{}/{}", TEST_PATH, "server-1.key"),
                            // removed legacy ca field
                        },
                        ..Default::default()
                    },
                    client_ca: CaSource::Pem { data: ca_pem },
                    ..Default::default()
                }
            }) as Box<dyn Fn() -> TlsServerConfig>,
            false,
            ErrorMessage::Empty,
            false,
        ),
        (
            "test-client-ca-wrong-pem",
            Box::new(|| TlsServerConfig {
                config: Config {
                    source: TlsSource::File {
                        cert: format!("{}/{}", TEST_PATH, "server-1.crt"),
                        key: format!("{}/{}", TEST_PATH, "server-1.key"),
                        // removed legacy ca field
                    },
                    ..Default::default()
                },
                client_ca: CaSource::Pem {
                    data: String::from(
                        "-----BEGIN CERTIFICATE-----\nwrong\n-----END CERTIFICATE-----",
                    ),
                },
                ..Default::default()
            }) as Box<dyn Fn() -> TlsServerConfig>,
            true,
            ErrorMessage::Is(String::from("invalid pem format")),
            false,
        ),
    ];

    for (test_name, server_config, error_expected, error_message, print_error) in tests.iter() {
        let config = (**server_config)();

        test_load_rustls_config(
            test_name,
            &config,
            error_expected,
            error_message,
            print_error,
        )
        .await;
    }
}
