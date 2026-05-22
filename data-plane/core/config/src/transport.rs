// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
}
