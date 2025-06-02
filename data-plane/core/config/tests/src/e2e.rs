// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use tonic::{metadata::KeyAndValueRef, Request, Response, Status};
use tracing::info;

use slim_config_grpc::client::ClientConfig;
use slim_config_grpc::testutils::helloworld::greeter_server::Greeter;
use slim_config_grpc::testutils::helloworld::{HelloReply, HelloRequest};

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

    use super::*;
    use slim_config_auth::basic::Config as BasicAuthConfig;
    use slim_config_auth::bearer::Config as BearerAuthConfig;
    use slim_config_grpc::client::AuthenticationConfig as ClientAuthenticationConfig;
    use slim_config_grpc::server::AuthenticationConfig as ServerAuthenticationConfig;
    use slim_config_tls::client::TlsClientConfig;
    use slim_config_tls::server::TlsServerConfig;
    use tracing::debug;
    use tracing::info;
    use tracing_test::traced_test;

    // use slim_config_grpc::headers_middleware::SetRequestHeader;
    use slim_config_grpc::testutils::helloworld::greeter_client::GreeterClient;
    use slim_config_grpc::testutils::helloworld::greeter_server::GreeterServer;
    use slim_config_grpc::testutils::helloworld::HelloRequest;
    use slim_config_grpc::{client::ClientConfig, server::ServerConfig};

    static TEST_DATA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");

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
                    .with_ca_file(&(TEST_DATA_PATH.to_string() + "/tls/ca.crt")),
            );

        // create server config
        let data_dir = std::path::PathBuf::from_iter([TEST_DATA_PATH]);
        let cert = std::fs::read_to_string(data_dir.join("tls/server.crt")).unwrap();
        let key = std::fs::read_to_string(data_dir.join("tls/server.key")).unwrap();
        let server_config = ServerConfig::with_endpoint("[::1]:50052").with_tls_settings(
            TlsServerConfig::new()
                .with_insecure(false)
                .with_cert_pem(&cert)
                .with_key_pem(&key),
        );

        // run grpc server and client
        setup_client_and_server(client_config, server_config).await
    }

    #[tokio::test]
    #[traced_test]
    async fn test_tls_auth_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config = ClientConfig::with_endpoint("https://[::1]:50053")
            .with_headers(HashMap::from([(
                "x-custom-header".to_string(),
                "custom-value".to_string(),
            )]))
            .with_tls_setting(
                TlsClientConfig::new()
                    .with_insecure(false)
                    .with_insecure_skip_verify(true)
                    .with_tls_version("tls1.3")
                    .with_ca_file(&(TEST_DATA_PATH.to_string() + "/tls/ca.crt")),
            )
            .with_auth(ClientAuthenticationConfig::Basic(BasicAuthConfig::new(
                "user", "password",
            )));

        // create server config
        let data_dir = std::path::PathBuf::from_iter([TEST_DATA_PATH]);
        let cert = std::fs::read_to_string(data_dir.join("tls/server.crt")).unwrap();
        let key = std::fs::read_to_string(data_dir.join("tls/server.key")).unwrap();
        let server_config = ServerConfig::with_endpoint("[::1]:50053")
            .with_tls_settings(
                TlsServerConfig::new()
                    .with_insecure(false)
                    .with_cert_pem(&cert)
                    .with_key_pem(&key),
            )
            .with_auth(ServerAuthenticationConfig::Basic(BasicAuthConfig::new(
                "user", "password",
            )));

        // run grpc server and client
        setup_client_and_server(client_config.clone(), server_config).await;

        // create a new client with wrong credentials
        let channel = client_config
            .with_auth(ClientAuthenticationConfig::Basic(BasicAuthConfig::new(
                "user", "wrong",
            )))
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
    async fn test_tls_bearer_auth_grpc_configuration() {
        // create a client configuration and derive a channel from it
        let client_config = ClientConfig::with_endpoint("https://[::1]:50054")
            .with_headers(HashMap::from([(
                "x-custom-header".to_string(),
                "custom-value".to_string(),
            )]))
            .with_tls_setting(
                TlsClientConfig::new()
                    .with_insecure(false)
                    .with_insecure_skip_verify(true)
                    .with_tls_version("tls1.3")
                    .with_ca_file(&(TEST_DATA_PATH.to_string() + "/tls/ca.crt")),
            )
            .with_auth(ClientAuthenticationConfig::Bearer(BearerAuthConfig::new(
                "token",
            )));

        // create server config
        let data_dir = std::path::PathBuf::from_iter([TEST_DATA_PATH]);
        let cert = std::fs::read_to_string(data_dir.join("tls/server.crt")).unwrap();
        let key = std::fs::read_to_string(data_dir.join("tls/server.key")).unwrap();
        let server_config = ServerConfig::with_endpoint("[::1]:50054")
            .with_tls_settings(
                TlsServerConfig::new()
                    .with_insecure(false)
                    .with_cert_pem(&cert)
                    .with_key_pem(&key),
            )
            .with_auth(ServerAuthenticationConfig::Bearer(BearerAuthConfig::new(
                "token",
            )));

        // run grpc server and client
        setup_client_and_server(client_config.clone(), server_config).await;

        // create a new client with wrong credentials
        let channel = client_config
            .with_auth(ClientAuthenticationConfig::Bearer(BearerAuthConfig::new(
                "wrong",
            )))
            .to_channel()
            .unwrap();

        let mut client = GreeterClient::new(channel);

        // send request to server
        let request = tonic::Request::new(HelloRequest { name: "wee".into() });

        // wait for response
        let response = client.say_hello(request).await;
        assert!(response.is_err(), "error: {:?}", response.err());
    }
}
