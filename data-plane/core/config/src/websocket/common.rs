// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fastwebsockets::WebSocket;
use hyper::Request;
use hyper::body::Incoming;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use slim_auth::traits::{TokenProvider, Verifier};
use tracing::warn;

use crate::client::{AuthenticationConfig as ClientAuthConfig, ClientConfig};
use crate::grpc::errors::ConfigError;
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
    }
}

impl ServerHandshakeAuth {
    pub async fn authorize(&self, request: &Request<Incoming>) -> bool {
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
