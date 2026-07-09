// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use crate::errors::ConfigError;

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use fastwebsockets::WebSocket;
        use hyper::upgrade::Upgraded;
        use hyper_util::rt::TokioIo;

        pub type UpgradedWebSocket = WebSocket<TokioIo<Upgraded>>;
    }
}

#[derive(Debug, Clone)]
pub struct WebSocketEndpoint {
    pub uri: http::Uri,
    pub secure: bool,
    pub host: String,
    pub authority: String,
    pub port: u16,
    pub path: String,
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
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn request_uri(&self) -> Result<http::Uri, ConfigError> {
        self.build_uri(if self.secure { "wss" } else { "ws" })
    }

    /// Like [`Self::request_uri`] but uses `http://` / `https://` schemes
    /// instead of `ws://` / `wss://`. Required when driving the upgrade via
    /// `hyper_util::client::legacy::Client`, which rejects WebSocket schemes.
    pub fn http_request_uri(&self) -> Result<http::Uri, ConfigError> {
        self.build_uri(if self.secure { "https" } else { "http" })
    }

    fn build_uri(&self, scheme: &str) -> Result<http::Uri, ConfigError> {
        let mut uri = format!("{}://{}{}", scheme, self.authority, self.path);

        if let Some(existing_query) = self.uri.query() {
            uri.push('?');
            uri.push_str(existing_query);
        }

        http::Uri::from_str(&uri).map_err(ConfigError::from)
    }
}

/// Extract a query parameter by name from a raw URI query string,
/// percent-decoding both key and value. Used by the server-side
/// `QueryTokenToAuthHeaderLayer` to accept tokens from clients that cannot
/// set HTTP headers on the WebSocket upgrade (browsers).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn extract_query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;

    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();

        // Percent-decode both key and value. Clients that send tokens
        // containing `=` (base64 padding) or `+` MUST percent-encode them
        // (`%3D`, `%2B`); otherwise the raw bytes would corrupt the
        // parameter or, worse, be interpreted as separators.
        let decoded_key = percent_encoding::percent_decode_str(key)
            .decode_utf8()
            .ok()?;

        if decoded_key == name && !value.is_empty() {
            let decoded_value = percent_encoding::percent_decode_str(value)
                .decode_utf8()
                .ok()?
                .into_owned();
            if decoded_value.is_empty() {
                return None;
            }
            return Some(decoded_value);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_websocket_endpoint_socket_address_ipv6_bracketed() {
        // Authority `[::1]:9000` — host must be re-bracketed for connect().
        let ep = WebSocketEndpoint::parse("ws://[::1]:9000/").expect("parse");
        assert_eq!(ep.socket_address(), "[::1]:9000");
    }

    #[test]
    fn test_websocket_endpoint_socket_address_ipv4() {
        let ep = WebSocketEndpoint::parse("ws://127.0.0.1:8080/").expect("parse");
        assert_eq!(ep.socket_address(), "127.0.0.1:8080");
    }

    #[test]
    fn test_websocket_endpoint_parse_invalid_scheme() {
        assert!(WebSocketEndpoint::parse("http://example.com").is_err());
        assert!(WebSocketEndpoint::parse("://nohost").is_err());
    }

    #[test]
    fn test_request_uri_preserves_existing_query() {
        let ep = WebSocketEndpoint::parse("ws://example.com/path?k=v").expect("parse");
        let uri = ep.request_uri().expect("uri");
        assert_eq!(uri.to_string(), "ws://example.com/path?k=v");
    }

    #[test]
    fn test_request_uri_no_query() {
        let ep = WebSocketEndpoint::parse("ws://example.com/p").expect("parse");
        let uri = ep.request_uri().expect("uri");
        assert_eq!(uri.to_string(), "ws://example.com/p");
    }

    #[test]
    fn test_http_request_uri_translates_scheme() {
        let ep = WebSocketEndpoint::parse("wss://example.com/p").expect("parse");
        let uri = ep.http_request_uri().expect("uri");
        assert_eq!(uri.to_string(), "https://example.com/p");
    }

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

    #[test]
    fn test_extract_query_param_percent_decodes_value() {
        // `+` and `=` encoded as `%2B` and `%3D` (e.g. base64-padded token).
        assert_eq!(
            extract_query_param(Some("token=ab%2Bcd%3D%3D"), "token"),
            Some("ab+cd==".to_string())
        );
    }

    #[test]
    fn test_extract_query_param_percent_decodes_key() {
        assert_eq!(
            extract_query_param(Some("to%6Ben=value"), "token"),
            Some("value".to_string())
        );
    }
}
