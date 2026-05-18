// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fastwebsockets::WebSocket;
use hyper::Request;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use slim_auth::traits::{TokenProvider, Verifier};
use tracing::warn;

use crate::client::{AuthenticationConfig as ClientAuthConfig, ClientConfig};
use crate::errors::ConfigError;
use crate::server::{AuthenticationConfig as ServerAuthConfig, ServerConfig};

pub type UpgradedWebSocket = WebSocket<TokioIo<Upgraded>>;

#[derive(Debug, Clone)]
pub struct WebSocketEndpoint {
    pub uri: http::Uri,
    pub secure: bool,
    pub host: String,
    pub authority: String,
    pub port: u16,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct ClientHandshakeAuth {
    pub authorization_header: Option<String>,
    pub bearer_token: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ServerHandshakeAuth {
    None,
    Basic { username: String, password: String },
    Jwt { config: crate::auth::jwt::Config },
}

impl WebSocketEndpoint {
    pub fn parse(endpoint: &str) -> Result<Self, ConfigError> {
        let uri = http::Uri::from_str(endpoint)?;

        let secure = match uri.scheme_str() {
            Some("ws") => false,
            Some("wss") => true,
            _ => return Err(ConfigError::InvalidWebSocketEndpointScheme),
        };

        let authority = uri
            .authority()
            .ok_or(ConfigError::InvalidWebSocketEndpointScheme)?
            .as_str()
            .to_string();
        let host = uri
            .host()
            .ok_or(ConfigError::InvalidWebSocketEndpointScheme)?
            .to_string();
        let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });

        let mut path = uri.path().to_string();
        if path.is_empty() {
            path.push('/');
        }

        Ok(Self {
            uri,
            secure,
            host,
            authority,
            port,
            path,
        })
    }

    pub fn socket_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn request_uri(&self, query_param: Option<(&str, &str)>) -> Result<http::Uri, ConfigError> {
        let mut uri = format!(
            "{}://{}{}",
            if self.secure { "wss" } else { "ws" },
            self.authority,
            self.path,
        );

        if let Some(existing_query) = self.uri.query() {
            uri.push('?');
            uri.push_str(existing_query);
        }

        if let Some((key, value)) = query_param
            && !key.is_empty()
            && !value.is_empty()
        {
            if self.uri.query().is_some() {
                uri.push('&');
            } else {
                uri.push('?');
            }
            uri.push_str(key);
            uri.push('=');
            uri.push_str(value);
        }

        http::Uri::from_str(&uri).map_err(ConfigError::from)
    }
}

pub async fn build_client_handshake_auth(
    config: &ClientConfig,
) -> Result<ClientHandshakeAuth, ConfigError> {
    match &config.auth {
        ClientAuthConfig::None => Ok(ClientHandshakeAuth::default()),
        ClientAuthConfig::Basic(basic) => {
            let encoded = BASE64_STANDARD.encode(format!(
                "{}:{}",
                basic.username(),
                basic.password().as_str()
            ));
            Ok(ClientHandshakeAuth {
                authorization_header: Some(format!("Basic {}", encoded)),
                bearer_token: None,
            })
        }
        ClientAuthConfig::StaticJwt(static_jwt) => {
            let mut provider = static_jwt.build_static_token_provider()?;
            provider.initialize().await?;
            let token = provider.get_token()?;
            Ok(ClientHandshakeAuth {
                authorization_header: Some(format!("Bearer {}", token)),
                bearer_token: Some(token),
            })
        }
        ClientAuthConfig::Jwt(jwt) => {
            let mut provider = jwt.get_provider()?;
            provider.initialize().await?;
            let token = provider.get_token()?;
            Ok(ClientHandshakeAuth {
                authorization_header: Some(format!("Bearer {}", token)),
                bearer_token: Some(token),
            })
        }
        #[cfg(not(target_family = "windows"))]
        ClientAuthConfig::Spire(_) => Err(ConfigError::WebSocketTlsConfiguration),
    }
}

pub fn build_server_handshake_auth(config: &ServerConfig) -> ServerHandshakeAuth {
    match &config.auth {
        ServerAuthConfig::None => ServerHandshakeAuth::None,
        ServerAuthConfig::Basic(basic) => ServerHandshakeAuth::Basic {
            username: basic.username().to_string(),
            password: basic.password().as_str().to_string(),
        },
        ServerAuthConfig::Jwt(jwt) => ServerHandshakeAuth::Jwt {
            config: jwt.clone(),
        },
        #[cfg(not(target_family = "windows"))]
        ServerAuthConfig::Spire(_) => ServerHandshakeAuth::None,
    }
}

impl ServerHandshakeAuth {
    pub async fn authorize<B>(&self, request: &Request<B>) -> bool {
        match self {
            ServerHandshakeAuth::None => true,
            ServerHandshakeAuth::Basic { username, password } => {
                let auth = match request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                {
                    Some(value) => value,
                    None => return false,
                };

                let Some(encoded) = auth.strip_prefix("Basic ") else {
                    return false;
                };

                let decoded = match BASE64_STANDARD.decode(encoded.as_bytes()) {
                    Ok(bytes) => bytes,
                    Err(_) => return false,
                };

                let credentials = match std::str::from_utf8(&decoded) {
                    Ok(credentials) => credentials,
                    Err(_) => return false,
                };

                credentials == format!("{username}:{password}")
            }
            ServerHandshakeAuth::Jwt { config } => {
                let token = request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|header| header.strip_prefix("Bearer "))
                    .map(str::to_string)
                    .or_else(|| extract_query_param(request.uri().query(), "token"));

                let Some(token) = token else {
                    return false;
                };

                let mut verifier = match config.get_verifier() {
                    Ok(verifier) => verifier,
                    Err(err) => {
                        warn!(error = %err, "failed to create websocket JWT verifier");
                        return false;
                    }
                };

                if verifier.initialize().await.is_err() {
                    return false;
                }

                verifier.verify(token).await.is_ok()
            }
        }
    }
}

fn extract_query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;

    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();

        if key == name && !value.is_empty() {
            return Some(value.to_string());
        }
    }

    None
}

// =====================================================================
// Tests
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use bytes::Bytes;
    use http::Uri;
    use http_body_util::Empty;
    use hyper::Request;
    use slim_auth::jwt::{Algorithm, KeyData, KeyFormat};
    use slim_auth::traits::TokenProvider;
    use std::time::Duration as StdDuration;

    use crate::auth::jwt::{Claims as JwtClaims, Config as JwtConfig, JwtKey};

    // ----- Test constants. Not real credentials. ------------------------
    // pragma: allowlist secret
    const TEST_USER: &str = "user";
    // pragma: allowlist secret
    const TEST_PASS: &str = "pass";
    // pragma: allowlist secret -- HS256 shared secret for unit tests only
    const TEST_JWT_SECRET: &str = "websocket-tests-shared-secret";

    fn empty_request(uri: &str) -> Request<Empty<Bytes>> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Empty::<Bytes>::new())
            .expect("build request")
    }

    fn request_with_header(uri: &str, header: &str, value: String) -> Request<Empty<Bytes>> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(header, value)
            .body(Empty::<Bytes>::new())
            .expect("build request")
    }

    fn hs256_key_data() -> KeyData {
        // pragma: allowlist secret
        KeyData::Data(TEST_JWT_SECRET.to_string())
    }

    fn jwt_signer_config() -> JwtConfig {
        JwtConfig::new(
            JwtClaims::default()
                .with_issuer("slim-tests")
                .with_audience(&["slim-tests".to_string()])
                .with_subject("ws-client"),
            StdDuration::from_secs(60),
            JwtKey::Encoding(slim_auth::jwt::Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: hs256_key_data(),
            }),
        )
    }

    fn jwt_verifier_config() -> JwtConfig {
        JwtConfig::new(
            JwtClaims::default()
                .with_issuer("slim-tests")
                .with_audience(&["slim-tests".to_string()])
                .with_subject("ws-client"),
            StdDuration::from_secs(60),
            JwtKey::Decoding(slim_auth::jwt::Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: hs256_key_data(),
            }),
        )
    }

    async fn issue_jwt_token(config: &JwtConfig) -> String {
        let mut provider = config.get_provider().expect("provider");
        provider.initialize().await.expect("initialize");
        provider.get_token().expect("token")
    }

    // ---------------- WebSocketEndpoint::parse ----------------

    #[test]
    fn test_websocket_endpoint_parse_ws() {
        let ep = WebSocketEndpoint::parse("ws://example.com:8080/path").expect("parse");
        assert!(!ep.secure);
        assert_eq!(ep.host, "example.com");
        assert_eq!(ep.port, 8080);
        assert_eq!(ep.path, "/path");
        assert_eq!(ep.socket_address(), "example.com:8080");
    }

    #[test]
    fn test_websocket_endpoint_parse_wss() {
        let ep = WebSocketEndpoint::parse("wss://example.com/secure").expect("parse");
        assert!(ep.secure);
        assert_eq!(ep.port, 443, "wss should default to 443");
    }

    #[test]
    fn test_websocket_endpoint_default_port_ws() {
        let ep = WebSocketEndpoint::parse("ws://example.com/path").expect("parse");
        assert_eq!(ep.port, 80);
    }

    #[test]
    fn test_websocket_endpoint_default_path() {
        let ep = WebSocketEndpoint::parse("ws://example.com").expect("parse");
        assert_eq!(ep.path, "/");
    }

    #[test]
    fn test_websocket_endpoint_parse_invalid_scheme() {
        assert!(WebSocketEndpoint::parse("http://example.com").is_err());
        assert!(WebSocketEndpoint::parse("://nohost").is_err());
    }

    #[test]
    fn test_request_uri_appends_query_param() {
        let ep = WebSocketEndpoint::parse("ws://example.com/path").expect("parse");
        let uri = ep.request_uri(Some(("token", "abc"))).expect("uri");
        assert_eq!(uri.to_string(), "ws://example.com/path?token=abc");
    }

    #[test]
    fn test_request_uri_merges_with_existing_query() {
        let ep = WebSocketEndpoint::parse("ws://example.com/path?k=v").expect("parse");
        let uri = ep.request_uri(Some(("token", "abc"))).expect("uri");
        assert_eq!(uri.to_string(), "ws://example.com/path?k=v&token=abc");
    }

    #[test]
    fn test_request_uri_no_query() {
        let ep = WebSocketEndpoint::parse("ws://example.com/p").expect("parse");
        let uri = ep.request_uri(None).expect("uri");
        assert_eq!(uri.to_string(), "ws://example.com/p");
    }

    // ---------------- extract_query_param ----------------

    #[test]
    fn test_extract_query_param_single() {
        assert_eq!(
            extract_query_param(Some("token=abc"), "token"),
            Some("abc".to_string())
        );
    }

    #[test]
    fn test_extract_query_param_multiple() {
        assert_eq!(
            extract_query_param(Some("a=1&token=xyz&b=2"), "token"),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn test_extract_query_param_missing() {
        assert_eq!(extract_query_param(Some("a=1&b=2"), "token"), None);
    }

    #[test]
    fn test_extract_query_param_none_query() {
        assert_eq!(extract_query_param(None, "token"), None);
    }

    #[test]
    fn test_extract_query_param_empty_value() {
        assert_eq!(
            extract_query_param(Some("token=&other=1"), "token"),
            None,
            "empty value should not be extracted"
        );
    }

    #[test]
    fn test_extract_query_param_no_equals() {
        assert_eq!(
            extract_query_param(Some("token&other=1"), "token"),
            None,
            "param without value should not be extracted"
        );
    }

    #[test]
    fn test_extract_query_param_first_match_wins() {
        assert_eq!(
            extract_query_param(Some("token=first&token=second"), "token"),
            Some("first".to_string())
        );
    }

    // ---------------- ServerHandshakeAuth::authorize: None / Basic ----------------

    #[tokio::test]
    async fn test_server_handshake_auth_none() {
        let auth = ServerHandshakeAuth::None;
        assert!(auth.authorize(&empty_request("/")).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_success() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        let encoded = BASE64_STANDARD.encode(format!("{TEST_USER}:{TEST_PASS}"));
        let req = request_with_header("/", "Authorization", format!("Basic {encoded}"));
        assert!(auth.authorize(&req).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_wrong_credentials() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        let encoded = BASE64_STANDARD.encode(format!("{TEST_USER}:wrong"));
        let req = request_with_header("/", "Authorization", format!("Basic {encoded}"));
        assert!(!auth.authorize(&req).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_missing_header() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        assert!(!auth.authorize(&empty_request("/")).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_invalid_base64() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        let req = request_with_header("/", "Authorization", "Basic !!not-base64!!".to_string());
        assert!(!auth.authorize(&req).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_invalid_prefix() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        let encoded = BASE64_STANDARD.encode(format!("{TEST_USER}:{TEST_PASS}"));
        let req = request_with_header("/", "Authorization", format!("Bearer {encoded}"));
        assert!(!auth.authorize(&req).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_basic_non_utf8() {
        let auth = ServerHandshakeAuth::Basic {
            username: TEST_USER.to_string(),
            password: TEST_PASS.to_string(), // pragma: allowlist secret
        };
        let encoded = BASE64_STANDARD.encode([0xff, 0xfe, 0xfd]);
        let req = request_with_header("/", "Authorization", format!("Basic {encoded}"));
        assert!(!auth.authorize(&req).await);
    }

    // ---------------- ServerHandshakeAuth::authorize: JWT ----------------

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_bearer_success() {
        let token = issue_jwt_token(&jwt_signer_config()).await;
        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };

        let req = request_with_header("/", "Authorization", format!("Bearer {token}"));
        assert!(
            auth.authorize(&req).await,
            "valid JWT in Bearer header should authorize"
        );
    }

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_bearer_invalid_token() {
        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };
        let req = request_with_header(
            "/",
            "Authorization",
            "Bearer not.a.jwt".to_string(), // pragma: allowlist secret
        );
        assert!(!auth.authorize(&req).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_missing_token() {
        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };
        assert!(!auth.authorize(&empty_request("/")).await);
    }

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_wrong_signing_secret() {
        let other_signer = JwtConfig::new(
            JwtClaims::default()
                .with_issuer("slim-tests")
                .with_audience(&["slim-tests".to_string()])
                .with_subject("ws-client"),
            StdDuration::from_secs(60),
            JwtKey::Encoding(slim_auth::jwt::Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                // pragma: allowlist secret
                key: KeyData::Data("different-secret".to_string()),
            }),
        );
        let token = issue_jwt_token(&other_signer).await;

        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };
        let req = request_with_header("/", "Authorization", format!("Bearer {token}"));
        assert!(
            !auth.authorize(&req).await,
            "JWT signed with a different secret must be rejected"
        );
    }

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_query_param_success() {
        let token = issue_jwt_token(&jwt_signer_config()).await;
        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };

        let uri = Uri::try_from(format!("/?token={token}")).expect("uri");
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Empty::<Bytes>::new())
            .expect("build request");
        assert!(
            auth.authorize(&req).await,
            "valid JWT in ?token= query param should authorize"
        );
    }

    #[tokio::test]
    async fn test_server_handshake_auth_jwt_query_param_invalid() {
        let auth = ServerHandshakeAuth::Jwt {
            config: jwt_verifier_config(),
        };
        let req = empty_request("/?token=not.a.jwt"); // pragma: allowlist secret
        assert!(!auth.authorize(&req).await);
    }

    // ---------------- build_*_handshake_auth ----------------

    #[tokio::test]
    async fn test_build_server_handshake_auth_none() {
        let cfg = ServerConfig::with_endpoint("ws://127.0.0.1:0");
        assert!(matches!(
            build_server_handshake_auth(&cfg),
            ServerHandshakeAuth::None
        ));
    }

    #[tokio::test]
    async fn test_build_server_handshake_auth_basic() {
        let cfg = ServerConfig::with_endpoint("ws://127.0.0.1:0").with_auth(
            ServerAuthConfig::Basic(crate::auth::basic::Config::new(TEST_USER, TEST_PASS)),
        );
        match build_server_handshake_auth(&cfg) {
            ServerHandshakeAuth::Basic { username, password } => {
                assert_eq!(username, TEST_USER);
                assert_eq!(password, TEST_PASS);
            }
            _ => panic!("expected Basic"),
        }
    }

    #[tokio::test]
    async fn test_build_client_handshake_auth_none() {
        let cfg = ClientConfig::with_endpoint("ws://127.0.0.1:0");
        let auth = build_client_handshake_auth(&cfg).await.expect("ok");
        assert!(auth.authorization_header.is_none());
        assert!(auth.bearer_token.is_none());
    }

    #[tokio::test]
    async fn test_build_client_handshake_auth_basic() {
        let cfg = ClientConfig::with_endpoint("ws://127.0.0.1:0").with_auth(
            ClientAuthConfig::Basic(crate::auth::basic::Config::new(TEST_USER, TEST_PASS)),
        );
        let auth = build_client_handshake_auth(&cfg).await.expect("ok");
        let header = auth.authorization_header.expect("header");
        assert!(header.starts_with("Basic "));
        let encoded = header.strip_prefix("Basic ").unwrap();
        let decoded = BASE64_STANDARD.decode(encoded).expect("decode");
        assert_eq!(decoded, format!("{TEST_USER}:{TEST_PASS}").as_bytes());
    }

    #[tokio::test]
    async fn test_build_client_handshake_auth_jwt_emits_bearer_token() {
        let cfg = ClientConfig::with_endpoint("ws://127.0.0.1:0")
            .with_auth(ClientAuthConfig::Jwt(jwt_signer_config()));
        let auth = build_client_handshake_auth(&cfg).await.expect("ok");
        let token = auth.bearer_token.expect("bearer token");
        assert_eq!(
            auth.authorization_header.as_deref(),
            Some(format!("Bearer {token}").as_str())
        );
    }
}
