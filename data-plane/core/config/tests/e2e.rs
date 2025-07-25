// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use tonic::{Request, Response, Status, metadata::KeyAndValueRef};
use tracing::info;

use slim_config::grpc::client::ClientConfig;
use slim_config::testutils::helloworld::greeter_server::Greeter;
use slim_config::testutils::helloworld::{HelloReply, HelloRequest};

#[derive(Default)]
pub struct TestGreeter {
    // Add a field to hold the configuration
    config: ClientConfig,
}

impl TestGreeter {
    pub fn new(config: ClientConfig) -> Self {
        Self { config }
    }
}

#[tonic::async_trait]
impl Greeter for TestGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        info!("Got a request from {:?}", request.remote_addr());

        // print request headers and make sure the one we set in the configuration are there
        for key_and_value in request.metadata().iter() {
            match key_and_value {
                KeyAndValueRef::Ascii(ref key, ref value) => {
                    info!("Ascii: {:?}: {:?}", key, value)
                }
                KeyAndValueRef::Binary(ref key, ref value) => {
                    info!("Binary: {:?}: {:?}", key, value)
                }
            }
        }

        // make sure the custom headers we set in the configuration are there
        for (key, value) in self.config.headers.iter() {
            // check that the additional headers we set are there
            let header = request.metadata().get(key);
            assert!(header.is_some());

            // check that the value is correct
            let header = header.unwrap();
            assert_eq!(header.to_str().unwrap(), value);
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        Ok(Response::new(reply))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;
    use slim_auth::jwt::Algorithm;
    use slim_auth::jwt::KeyFormat;
    use slim_config::auth::basic::Config as BasicAuthConfig;
    use slim_config::auth::bearer::Config as BearerAuthConfig;
    use slim_config::auth::jwt::Config as JwtAuthConfig;
    use slim_config::auth::jwt::JwtKey;
    use slim_config::grpc::client::AuthenticationConfig as ClientAuthenticationConfig;
    use slim_config::grpc::server::AuthenticationConfig as ServerAuthenticationConfig;
    use slim_config::tls::client::TlsClientConfig;
    use slim_config::tls::server::TlsServerConfig;
    use tracing::debug;
    use tracing::info;
    use tracing_test::traced_test;

    // use slim_config_grpc::headers_middleware::SetRequestHeader;
    use slim_auth::jwt::{Key, KeyData};
    use slim_auth::testutils::setup_test_jwt_resolver;
    use slim_config::grpc::{client::ClientConfig, server::ServerConfig};
    use slim_config::testutils::helloworld::HelloRequest;
    use slim_config::testutils::helloworld::greeter_client::GreeterClient;
    use slim_config::testutils::helloworld::greeter_server::GreeterServer;

    static TEST_DATA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

    async fn run_server(
        client_config: ClientConfig,
        server_config: ServerConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("GreeterServer listening on {}", server_config.endpoint);

        // instantiate server from config and start listening
        let greeter = TestGreeter::new(client_config);

        let ret = server_config.to_server_future(&[GreeterServer::new(greeter)]);
        assert!(ret.is_ok(), "error: {:?}", ret.err());

        let server_future = ret.unwrap();
        server_future.await?;

        Ok(())
    }

    async fn setup_client_and_server(client_config: ClientConfig, server_config: ServerConfig) {
        let _result = rustls::crypto::aws_lc_rs::default_provider().install_default();

        // run grpc server
        let client_config_clone = client_config.clone();
        let _server = tokio::spawn(async move {
            // clone the client configuration
            run_server(client_config_clone, server_config)
                .await
                .unwrap();
        });

        let channel_result = client_config.to_channel();

        // assert no error occurred
        assert!(channel_result.is_ok(), "error: {:?}", channel_result.err());

        // create a client using the channel
        let channel = channel_result.unwrap();
        let mut client = GreeterClient::new(channel);

        // send request to server
        let request = tonic::Request::new(HelloRequest {
            name: "slim".into(),
        });

        // wait for response
        let response = client.say_hello(request).await;
        assert!(response.is_ok(), "error: {:?}", response.err());

        // print response
        debug!("RESPONSE={:?}", response);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config = ClientConfig::with_endpoint("http://[::1]:50051")
            .with_headers(HashMap::from([(
                "x-custom-header".to_string(),
                "custom-value".to_string(),
            )]))
            .with_tls_setting(TlsClientConfig::new().with_insecure(true));

        // create server config
        let server_config = ServerConfig::with_endpoint("[::1]:50051")
            .with_tls_settings(TlsServerConfig::new().with_insecure(true));

        // run grpc server and client
        setup_client_and_server(client_config, server_config).await
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config = ClientConfig::with_endpoint("https://[::1]:50052")
            .with_headers(HashMap::from([(
                "x-custom-header".to_string(),
                "custom-value".to_string(),
            )]))
            .with_tls_setting(
                TlsClientConfig::new()
                    .with_insecure(false)
                    .with_insecure_skip_verify(true)
                    .with_tls_version("tls1.3")
                    .with_ca_file(&(TEST_DATA_PATH.to_string() + "/tls/ca-1.crt")),
            );

        // create server config
        let data_dir = std::path::PathBuf::from_iter([TEST_DATA_PATH]);
        let cert = std::fs::read_to_string(data_dir.join("tls/server-1.crt")).unwrap();
        let key = std::fs::read_to_string(data_dir.join("tls/server-1.key")).unwrap();
        let server_config = ServerConfig::with_endpoint("[::1]:50052").with_tls_settings(
            TlsServerConfig::new()
                .with_insecure(false)
                .with_cert_pem(&cert)
                .with_key_pem(&key),
        );

        // run grpc server and client
        setup_client_and_server(client_config, server_config).await
    }

    async fn test_grpc_auth(
        auth_client_config: ClientAuthenticationConfig,
        auth_server_config: ServerAuthenticationConfig,
        auth_wrong_client_config: ClientAuthenticationConfig,
        port: u16,
    ) {
        // create a client configuration and derive a channel from it
        let client_config = ClientConfig::with_endpoint(&format!("https://[::1]:{}", port))
            .with_headers(HashMap::from([(
                "x-custom-header".to_string(),
                "custom-value".to_string(),
            )]))
            .with_tls_setting(
                TlsClientConfig::new()
                    .with_insecure(false)
                    .with_insecure_skip_verify(true)
                    .with_tls_version("tls1.3")
                    .with_ca_file(&(TEST_DATA_PATH.to_string() + "/tls/ca-1.crt")),
            )
            .with_auth(auth_client_config);

        // create server config
        let data_dir = std::path::PathBuf::from_iter([TEST_DATA_PATH]);
        let cert = std::fs::read_to_string(data_dir.join("tls/server-1.crt")).unwrap();
        let key = std::fs::read_to_string(data_dir.join("tls/server-1.key")).unwrap();
        let server_config = ServerConfig::with_endpoint(&format!("[::1]:{}", port))
            .with_tls_settings(
                TlsServerConfig::new()
                    .with_insecure(false)
                    .with_cert_pem(&cert)
                    .with_key_pem(&key),
            )
            .with_auth(auth_server_config);

        // run grpc server and client
        setup_client_and_server(client_config.clone(), server_config).await;

        // create a new client with wrong credentials
        let channel = client_config
            .with_auth(auth_wrong_client_config)
            .to_channel()
            .unwrap();

        let mut client = GreeterClient::new(channel);

        // send request to server
        let request = tonic::Request::new(HelloRequest { name: "wee".into() });

        // wait for response
        let response = client.say_hello(request).await;
        assert!(response.is_err(), "error: {:?}", response.err());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_auth_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config =
            ClientAuthenticationConfig::Basic(BasicAuthConfig::new("user", "password"));

        // create server config
        let server_config =
            ServerAuthenticationConfig::Basic(BasicAuthConfig::new("user", "password"));

        // create wrong client config
        let wrong_client_config =
            ClientAuthenticationConfig::Basic(BasicAuthConfig::new("wrong", "password"));

        // run grpc server and client
        test_grpc_auth(client_config, server_config, wrong_client_config, 50054).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_bearer_auth_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config = ClientAuthenticationConfig::Bearer(BearerAuthConfig::new("test-token"));

        // create server config
        let server_config = ServerAuthenticationConfig::Bearer(BearerAuthConfig::new("test-token"));

        // create wrong client config
        let wrong_client_config =
            ClientAuthenticationConfig::Bearer(BearerAuthConfig::new("wrong-token"));

        // run grpc server and client
        test_grpc_auth(client_config, server_config, wrong_client_config, 50055).await;
    }

    async fn test_tls_jwt_grpc_configuration(
        key_client: Key,
        key_server: Key,
        key_client_wrong: Key,
        port: u16,
    ) {
        // Create JWT claims
        let claims = slim_config::auth::jwt::Claims::default()
            .with_issuer("test-issuer")
            .with_subject("test-subject")
            .with_audience(&["test-audience"]);

        let client_config = ClientAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims.clone(),
            Duration::from_secs(3600),
            JwtKey::Encoding(key_client.clone()),
        ));

        let server_config = ServerAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims.clone(),
            Duration::from_secs(3600),
            JwtKey::Decoding(key_server.clone()),
        ));

        let wring_client_config = ClientAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims,
            Duration::from_secs(3600),
            JwtKey::Encoding(key_client_wrong.clone()),
        ));

        // run grpc server and client
        test_grpc_auth(client_config, server_config, wring_client_config, port).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_hs256_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Str("secret-key".to_string()),
            },
            Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Str("secret-key".to_string()),
            },
            Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Str("wrong-key".to_string()),
            },
            50057,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa256_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::RS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::RS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::RS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50058,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa384_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::RS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::RS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::RS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50059,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa512_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::RS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::RS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::RS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50060,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ecdsa256_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::ES256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec256.pem"),
            },
            Key {
                algorithm: Algorithm::ES256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec256-public.pem"),
            },
            Key {
                algorithm: Algorithm::ES256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec256-wrong.pem"),
            },
            50061,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ecdsa384_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::ES384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec384.pem"),
            },
            Key {
                algorithm: Algorithm::ES384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec384-public.pem"),
            },
            Key {
                algorithm: Algorithm::ES384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/ec384-wrong.pem"),
            },
            50062,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ps256_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::PS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::PS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::PS256,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50063,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ps384_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::PS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::PS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::PS384,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50064,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ps512_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::PS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa.pem"),
            },
            Key {
                algorithm: Algorithm::PS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::PS512,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem"),
            },
            50065,
        )
        .await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_eddsa_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_grpc_configuration(
            Key {
                algorithm: Algorithm::EdDSA,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/eddsa.pem"),
            },
            Key {
                algorithm: Algorithm::EdDSA,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/eddsa-public.pem"),
            },
            Key {
                algorithm: Algorithm::EdDSA,
                format: KeyFormat::Pem,
                key: KeyData::File(TEST_DATA_PATH.to_string() + "/jwt/eddsa-wrong.pem"),
            },
            50066,
        )
        .await;
    }

    async fn test_tls_jwt_resolver_grpc_configuration(algorithm: Algorithm, port: u16) {
        let (test_key, mock_server, _alg_str) = setup_test_jwt_resolver(algorithm).await;

        // Create JWT claims
        let claims = slim_config::auth::jwt::Claims::default()
            .with_issuer(mock_server.uri())
            .with_subject("test-subject")
            .with_audience(&["test-audience"]);

        let client_config = ClientAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims.clone(),
            Duration::from_secs(3600),
            JwtKey::Encoding(Key {
                algorithm,
                format: KeyFormat::Pem,
                key: KeyData::Str(test_key.clone()),
            }),
        ));

        let server_config = ServerAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims.clone(),
            Duration::from_secs(3600),
            JwtKey::Autoresolve(true),
        ));

        let key_path = match algorithm {
            Algorithm::ES256 => TEST_DATA_PATH.to_string() + "/jwt/ec256-wrong.pem",
            Algorithm::ES384 => TEST_DATA_PATH.to_string() + "/jwt/ec384-wrong.pem",
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512 => TEST_DATA_PATH.to_string() + "/jwt/rsa-wrong.pem",
            Algorithm::EdDSA => TEST_DATA_PATH.to_string() + "/jwt/eddsa-wrong.pem",
            _ => panic!("Unsupported algorithm for test"),
        };

        let wring_client_config = ClientAuthenticationConfig::Jwt(JwtAuthConfig::new(
            claims,
            Duration::from_secs(3600),
            JwtKey::Encoding(Key {
                algorithm,
                format: KeyFormat::Pem,
                key: KeyData::File(key_path),
            }),
        ));

        // run grpc server and client
        test_grpc_auth(client_config, server_config, wring_client_config, port).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ecdsa256_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::ES256, 51111).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_ecdsa384_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::ES384, 51112).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa256_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::RS256, 51113).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa384_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::RS384, 51114).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsa512_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::RS512, 51115).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsap256_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::PS256, 51116).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsap384_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::PS384, 51117).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_rsap512_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::PS512, 51118).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_jwt_eddsa_autoresolve_grpc_configuration() {
        // Test RSA JWT configuration
        test_tls_jwt_resolver_grpc_configuration(Algorithm::EdDSA, 51119).await;
    }
}
