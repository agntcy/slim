// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

use crate::client::{AuthenticationConfig as ClientAuthConfig, ClientConfig};
use crate::grpc::errors::ConfigError;

#[derive(Debug, Clone)]
pub struct WebSocketEndpoint {
    pub uri: http::Uri,
    pub secure: bool,
    pub authority: String,
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

        let mut path = uri.path().to_string();
        if path.is_empty() {
            path.push('/');
        }

        Ok(Self {
            uri,
            secure,
            authority,
            path,
        })
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
        #[cfg(any(feature = "grpc", feature = "websocket-native"))]
        ClientAuthConfig::StaticJwt(static_jwt) => {
            use slim_auth::traits::TokenProvider;

            let mut provider = static_jwt.build_static_token_provider()?;
            provider.initialize().await?;
            let token = provider.get_token()?;
            Ok(ClientHandshakeAuth {
                authorization_header: Some(format!("Bearer {}", token)),
                bearer_token: Some(token),
            })
        }
        #[cfg(any(feature = "grpc", feature = "websocket-native"))]
        ClientAuthConfig::Jwt(jwt) => {
            use slim_auth::traits::TokenProvider;

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
