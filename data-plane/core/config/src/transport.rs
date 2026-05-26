// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::ConfigError;

/// Endpoint URI schemes recognized by `ClientConfig` and `ServerConfig`.
///
/// Kept in sync with the dispatch in [`TransportProtocol::from_endpoint`] and
/// the role-specific handling in `grpc/{client,server}.rs` and
/// `websocket/{client,server}.rs`.
const ALLOWED_ENDPOINT_SCHEMES: &[&str] = &["ws", "wss", "http", "https", "unix"];

/// Validates that an endpoint string carries a recognized URI scheme (or no
/// scheme, in which case it is treated as a bare `host:port` gRPC endpoint).
///
/// Shared by `ClientConfig::validate` and `ServerConfig::validate` so both
/// sides reject unknown schemes consistently.
///
/// The scheme is matched as a literal prefix rather than via `http::Uri`
/// parsing because authority-less endpoints such as `unix:///tmp/slim.sock`
/// are valid for gRPC but rejected by `http::Uri::from_str`.
pub fn validate_endpoint_scheme(endpoint: &str) -> Result<(), ConfigError> {
    let Some((scheme, _rest)) = endpoint.split_once("://") else {
        return Ok(());
    };
    if ALLOWED_ENDPOINT_SCHEMES.contains(&scheme) {
        Ok(())
    } else {
        Err(ConfigError::InvalidEndpointScheme)
    }
}

/// Transport protocol used by client and server dataplane configuration.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    /// gRPC transport (default).
    #[default]
    Grpc,
    /// Native websocket transport.
    Websocket,
}

impl TransportProtocol {
    /// Infer the transport protocol from an endpoint string by inspecting
    /// its URI scheme.
    ///
    /// Mapping:
    /// * `ws://`, `wss://`  → [`TransportProtocol::Websocket`]
    /// * everything else (`http://`, `https://`, `unix://`, bare `host:port`)
    ///   → [`TransportProtocol::Grpc`]
    pub fn from_endpoint(endpoint: &str) -> Self {
        match endpoint.split_once("://").map(|(s, _)| s) {
            Some("ws") | Some("wss") => TransportProtocol::Websocket,
            _ => TransportProtocol::Grpc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_grpc_from_http() {
        assert_eq!(
            TransportProtocol::from_endpoint("http://localhost:1234"),
            TransportProtocol::Grpc
        );
        assert_eq!(
            TransportProtocol::from_endpoint("https://localhost:1234"),
            TransportProtocol::Grpc
        );
    }

    #[test]
    fn infer_websocket_from_ws() {
        assert_eq!(
            TransportProtocol::from_endpoint("ws://localhost:1234"),
            TransportProtocol::Websocket
        );
        assert_eq!(
            TransportProtocol::from_endpoint("wss://localhost:1234"),
            TransportProtocol::Websocket
        );
    }

    #[test]
    fn infer_grpc_from_bare_host_port() {
        assert_eq!(
            TransportProtocol::from_endpoint("a.b.c.d:12345"),
            TransportProtocol::Grpc
        );
    }

    #[test]
    fn infer_grpc_from_unix() {
        assert_eq!(
            TransportProtocol::from_endpoint("unix:///tmp/slim.sock"),
            TransportProtocol::Grpc
        );
    }

    #[test]
    fn validate_endpoint_scheme_accepts_known_schemes() {
        for endpoint in [
            "host:1234",
            "http://host:1234",
            "https://host:1234",
            "ws://host:1234",
            "wss://host:1234",
            "unix:///tmp/slim.sock",
        ] {
            validate_endpoint_scheme(endpoint)
                .unwrap_or_else(|e| panic!("endpoint {endpoint} rejected: {e}"));
        }
    }

    #[test]
    fn validate_endpoint_scheme_rejects_unknown_scheme() {
        assert!(matches!(
            validate_endpoint_scheme("ftp://host:21"),
            Err(ConfigError::InvalidEndpointScheme)
        ));
    }
}
