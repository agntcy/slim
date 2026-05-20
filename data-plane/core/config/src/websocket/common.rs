// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use fastwebsockets::WebSocket;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use crate::errors::ConfigError;

pub type UpgradedWebSocket = WebSocket<TokioIo<Upgraded>>;

const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

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

    pub fn request_uri(&self, query_param: Option<(&str, &str)>) -> Result<http::Uri, ConfigError> {
        self.build_uri(if self.secure { "wss" } else { "ws" }, query_param)
    }

    /// Like [`Self::request_uri`] but uses `http://` / `https://` schemes
    /// instead of `ws://` / `wss://`. Required when driving the upgrade via
    /// `hyper_util::client::legacy::Client`, which rejects WebSocket schemes.
    pub fn http_request_uri(
        &self,
        query_param: Option<(&str, &str)>,
    ) -> Result<http::Uri, ConfigError> {
        self.build_uri(if self.secure { "https" } else { "http" }, query_param)
    }

    fn build_uri(
        &self,
        scheme: &str,
        query_param: Option<(&str, &str)>,
    ) -> Result<http::Uri, ConfigError> {
        let mut uri = format!("{}://{}{}", scheme, self.authority, self.path);

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
            // Percent-encode the user-supplied key/value before splicing
            // them into the URI. Without this, a value containing `&`,
            // `=`, `?`, `#`, or `+` would either break the URI parser or
            // smuggle an extra query parameter.
            for chunk in utf8_percent_encode(key, QUERY_ENCODE_SET) {
                uri.push_str(chunk);
            }
            uri.push('=');
            for chunk in utf8_percent_encode(value, QUERY_ENCODE_SET) {
                uri.push_str(chunk);
            }
        }

        http::Uri::from_str(&uri).map_err(ConfigError::from)
    }
}

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

    #[test]
    fn test_request_uri_percent_encodes_special_chars_in_value() {
        // Tokens may contain `=`, `+`, `&`, `?`, `#`, `/` etc. They must
        // not be smuggled into the URI as separators or fragments.
        let ep = WebSocketEndpoint::parse("ws://example.com/p").expect("parse");
        let uri = ep
            .request_uri(Some(("token", "a+b=c&d?e#f/g")))
            .expect("uri");
        let s = uri.to_string();
        assert!(
            s.starts_with("ws://example.com/p?token="),
            "unexpected uri prefix: {s}"
        );
        // None of these characters may appear unencoded in the query.
        for forbidden in ['+', '=', '&', '?', '#', '/'].iter() {
            let suffix = s.strip_prefix("ws://example.com/p?token=").unwrap();
            assert!(
                !suffix.contains(*forbidden),
                "value must be percent-encoded, got: {s}"
            );
        }
        // Spot-check a couple of expected encodings.
        assert!(s.contains("%2B"), "+ must be encoded as %2B: {s}");
        assert!(s.contains("%3D"), "= must be encoded as %3D: {s}");
        assert!(s.contains("%26"), "& must be encoded as %26: {s}");
    }

    #[test]
    fn test_request_uri_percent_encodes_special_chars_in_key() {
        let ep = WebSocketEndpoint::parse("ws://example.com/p").expect("parse");
        let uri = ep.request_uri(Some(("a=b", "v"))).expect("uri");
        let s = uri.to_string();
        assert_eq!(s, "ws://example.com/p?a%3Db=v");
    }

    #[test]
    fn test_request_uri_round_trip_through_extract_query_param() {
        // What we encode on the way out must decode cleanly on the way in.
        let ep = WebSocketEndpoint::parse("ws://example.com/p").expect("parse");
        let original = "ab+cd==/&?";
        let uri = ep.request_uri(Some(("token", original))).expect("uri");
        let query = uri.query().expect("query");
        assert_eq!(
            extract_query_param(Some(query), "token"),
            Some(original.to_string())
        );
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
