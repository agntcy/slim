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
use crate::grpc::errors::ConfigError;
use crate::server::AuthenticationConfig as ServerAuthConfig;

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
        #[cfg(all(feature = "native", not(target_family = "windows")))]
        ClientAuthConfig::Spire(spire) => {
            let mut provider = spire.create_provider()?;
            provider.initialize().await?;
            let token = provider.get_token()?;
            Ok(ClientHandshakeAuth {
                authorization_header: Some(format!("Bearer {}", token)),
                bearer_token: Some(token),
            })
        }
    }
}

pub async fn authorize_server_handshake(
    auth: &ServerAuthConfig,
    request: &Request<impl Send + Sync>,
) -> bool {
    // TODO(hackeramitkumar): Return structured rejection reasons so websocket server can
    // report auth failures through existing connection-state reporting hooks.
    match auth {
        ServerAuthConfig::None => true,
        ServerAuthConfig::Basic(basic) => {
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

            credentials == format!("{}:{}", basic.username(), basic.password().as_str())
        }
        ServerAuthConfig::Jwt(jwt) => {
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

            let mut verifier = match jwt.get_verifier() {
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
        #[cfg(all(feature = "native", not(target_family = "windows")))]
        ServerAuthConfig::Spire(spire) => {
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

            let mut verifier = match spire.create_provider() {
                Ok(verifier) => verifier,
                Err(err) => {
                    warn!(error = %err, "failed to create websocket SPIRE verifier");
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use http::header::AUTHORIZATION;

    use super::*;
    use crate::auth::basic::Config as BasicConfig;
    use crate::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use crate::auth::static_jwt::Config as StaticJwtConfig;
    use crate::client::AuthenticationConfig as ClientAuthenticationConfig;
    use crate::server::AuthenticationConfig as ServerAuthenticationConfig;
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
    use slim_auth::traits::TokenProvider;

    fn unique_temp_file(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{nanos}.jwt"))
    }

    fn hs256_key(secret: &str) -> Key {
        Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data(secret.to_string()),
        }
    }

    fn default_claims() -> Claims {
        Claims::default().with_subject("websocket-test-subject")
    }

    #[test]
    fn websocket_endpoint_parse_ws() {
        let endpoint = WebSocketEndpoint::parse("ws://example.com:8080/socket")
            .expect("ws endpoint should parse");
        assert!(!endpoint.secure);
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.authority, "example.com:8080");
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/socket");
        assert_eq!(endpoint.socket_address(), "example.com:8080");
    }

    #[test]
    fn websocket_endpoint_parse_wss_default_port() {
        let endpoint =
            WebSocketEndpoint::parse("wss://example.com/").expect("wss endpoint should parse");
        assert!(endpoint.secure);
        assert_eq!(endpoint.port, 443);
        assert_eq!(endpoint.path, "/");
        assert_eq!(endpoint.socket_address(), "example.com:443");
    }

    #[test]
    fn websocket_endpoint_parse_rejects_non_ws_scheme() {
        let err = WebSocketEndpoint::parse("http://example.com").expect_err("must fail");
        assert!(matches!(err, ConfigError::InvalidWebSocketEndpointScheme));
    }

    #[test]
    fn websocket_endpoint_request_uri_appends_query_param() {
        let endpoint =
            WebSocketEndpoint::parse("ws://localhost:9000/ws?existing=1").expect("must parse");
        let uri = endpoint
            .request_uri(Some(("token", "abc")))
            .expect("request uri should build");
        assert_eq!(
            uri.to_string(),
            "ws://localhost:9000/ws?existing=1&token=abc"
        );
    }

    #[test]
    fn websocket_endpoint_request_uri_ignores_empty_query_param() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:9000/ws").expect("must parse");
        let uri = endpoint
            .request_uri(Some(("token", "")))
            .expect("request uri should build");
        assert_eq!(uri.to_string(), "ws://localhost:9000/ws");
    }

    #[test]
    fn websocket_endpoint_request_uri_rejects_invalid_query_value() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:9000/ws").expect("must parse");
        let err = endpoint
            .request_uri(Some(("token", "a b")))
            .expect_err("must fail invalid URI");
        assert!(matches!(err, ConfigError::UriParse(_)));
    }

    #[test]
    fn extract_query_param_returns_expected_value() {
        let token = extract_query_param(Some("foo=bar&token=abc123"), "token");
        assert_eq!(token, Some("abc123".to_string()));
    }

    #[test]
    fn extract_query_param_returns_none_for_missing_or_empty() {
        assert_eq!(extract_query_param(Some("foo=bar"), "token"), None);
        assert_eq!(extract_query_param(Some("token="), "token"), None);
        assert_eq!(extract_query_param(None, "token"), None);
    }

    #[tokio::test]
    async fn build_client_handshake_auth_none() {
        let cfg = ClientConfig::with_endpoint("ws://localhost:46357");
        let auth = build_client_handshake_auth(&cfg)
            .await
            .expect("none auth should succeed");
        assert_eq!(auth.authorization_header, None);
        assert_eq!(auth.bearer_token, None);
    }

    #[tokio::test]
    async fn build_client_handshake_auth_basic() {
        // codeql[rust/hard-coded-cryptographic-value]
        let cfg = ClientConfig::with_endpoint("ws://localhost:46357").with_auth(
            ClientAuthenticationConfig::Basic(BasicConfig::new("alice", "secret")),
        );
        let auth = build_client_handshake_auth(&cfg)
            .await
            .expect("basic auth should succeed");
        assert_eq!(auth.bearer_token, None);
        assert_eq!(
            auth.authorization_header,
            Some("Basic YWxpY2U6c2VjcmV0".into())
        );
    }

    #[tokio::test]
    async fn build_client_handshake_auth_static_jwt() {
        let path = unique_temp_file("ws_static_token");
        std::fs::write(&path, "STATIC_TOKEN").expect("must write token file");
        let cfg = ClientConfig::with_endpoint("ws://localhost:46357").with_auth(
            ClientAuthenticationConfig::StaticJwt(StaticJwtConfig::with_file(
                path.to_string_lossy().to_string(),
            )),
        );

        let auth = build_client_handshake_auth(&cfg)
            .await
            .expect("static jwt auth should succeed");
        assert_eq!(auth.bearer_token.as_deref(), Some("STATIC_TOKEN"));
        assert_eq!(
            auth.authorization_header.as_deref(),
            Some("Bearer STATIC_TOKEN")
        );

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn build_client_handshake_auth_jwt() {
        let cfg = ClientConfig::with_endpoint("ws://localhost:46357").with_auth(
            ClientAuthenticationConfig::Jwt(JwtConfig::new(
                default_claims(),
                Duration::from_secs(60),
                JwtKey::Encoding(hs256_key("shared-secret")),
            )),
        );

        let auth = build_client_handshake_auth(&cfg)
            .await
            .expect("jwt auth should succeed");
        let token = auth
            .bearer_token
            .as_ref()
            .expect("jwt token should be present");
        assert!(!token.is_empty());
        assert_eq!(
            auth.authorization_header.as_deref(),
            Some(format!("Bearer {token}").as_str())
        );
    }

    #[tokio::test]
    async fn authorize_server_handshake_none_allows_request() {
        let req = Request::builder().uri("/ws").body(()).expect("request");
        let allowed = authorize_server_handshake(&ServerAuthenticationConfig::None, &req).await;
        assert!(allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_basic_accepts_valid_credentials() {
        let req = Request::builder()
            .uri("/ws")
            .header(AUTHORIZATION, "Basic YWxpY2U6c2VjcmV0")
            .body(())
            .expect("request");
        // codeql[rust/hard-coded-cryptographic-value]
        let auth = ServerAuthenticationConfig::Basic(BasicConfig::new("alice", "secret"));
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_basic_rejects_bad_header() {
        let req = Request::builder()
            .uri("/ws")
            .header(AUTHORIZATION, "Bearer token")
            .body(())
            .expect("request");
        // codeql[rust/hard-coded-cryptographic-value]
        let auth = ServerAuthenticationConfig::Basic(BasicConfig::new("alice", "secret"));
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(!allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_basic_rejects_bad_base64() {
        let req = Request::builder()
            .uri("/ws")
            .header(AUTHORIZATION, "Basic !!!not-base64!!!")
            .body(())
            .expect("request");
        // codeql[rust/hard-coded-cryptographic-value]
        let auth = ServerAuthenticationConfig::Basic(BasicConfig::new("alice", "secret"));
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(!allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_jwt_rejects_when_token_missing() {
        let auth = ServerAuthenticationConfig::Jwt(JwtConfig::new(
            default_claims(),
            Duration::from_secs(60),
            JwtKey::Decoding(hs256_key("shared-secret")),
        ));
        let req = Request::builder().uri("/ws").body(()).expect("request");
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(!allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_jwt_rejects_invalid_verifier_config() {
        let auth = ServerAuthenticationConfig::Jwt(JwtConfig::new(
            default_claims(),
            Duration::from_secs(60),
            JwtKey::Encoding(hs256_key("shared-secret")),
        ));
        let req = Request::builder()
            .uri("/ws")
            .header(AUTHORIZATION, "Bearer test-token")
            .body(())
            .expect("request");
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(!allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_jwt_accepts_valid_bearer_header() {
        let claims = default_claims();
        let signer_cfg = JwtConfig::new(
            claims.clone(),
            Duration::from_secs(60),
            JwtKey::Encoding(hs256_key("shared-secret")),
        );
        let mut signer = signer_cfg.get_provider().expect("signer");
        signer.initialize().await.expect("signer init");
        let token = signer.get_token().expect("token");

        let auth = ServerAuthenticationConfig::Jwt(JwtConfig::new(
            claims,
            Duration::from_secs(60),
            JwtKey::Decoding(hs256_key("shared-secret")),
        ));
        let req = Request::builder()
            .uri("/ws")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .expect("request");
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(allowed);
    }

    #[tokio::test]
    async fn authorize_server_handshake_jwt_accepts_query_param_token() {
        let claims = default_claims();
        let signer_cfg = JwtConfig::new(
            claims.clone(),
            Duration::from_secs(60),
            JwtKey::Encoding(hs256_key("shared-secret")),
        );
        let mut signer = signer_cfg.get_provider().expect("signer");
        signer.initialize().await.expect("signer init");
        let token = signer.get_token().expect("token");

        let auth = ServerAuthenticationConfig::Jwt(JwtConfig::new(
            claims,
            Duration::from_secs(60),
            JwtKey::Decoding(hs256_key("shared-secret")),
        ));
        let req = Request::builder()
            .uri(format!("/ws?token={token}"))
            .body(())
            .expect("request");
        let allowed = authorize_server_handshake(&auth, &req).await;
        assert!(allowed);
    }
}
