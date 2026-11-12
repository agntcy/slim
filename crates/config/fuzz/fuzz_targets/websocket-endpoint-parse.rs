// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parse arbitrary strings as a WebSocket endpoint.
//!
//! `WebSocketEndpoint::parse` reads the `endpoint` of a WebSocket client or
//! server config. Its result is used three ways: the server binds
//! `socket_address()`, the native client upgrades with `http_request_uri()`,
//! and the browser client connects with `request_uri()`.
//!
//! Invariants:
//! - no input panics;
//! - an accepted endpoint is `ws` or `wss` with a non-empty host, a path
//!   starting with `/`, and the explicit port or the scheme's default;
//! - an IP-literal host yields a `socket_address()` that parses as a socket
//!   address;
//! - both request URIs can be built for anything `parse` accepted, keep the
//!   authority, path and query, and parse back to the same endpoint.

#![no_main]

use std::net::{IpAddr, SocketAddr};

use libfuzzer_sys::fuzz_target;
use slim_config::websocket::common::WebSocketEndpoint;

fuzz_target!(|data: &[u8]| {
    // `parse` takes `&str`, so exercise UTF-8 boundaries by letting the
    // fuzzer produce invalid sequences and discarding those.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(ep) = WebSocketEndpoint::parse(s) else {
        return;
    };

    let scheme = if ep.secure { "wss" } else { "ws" };
    assert_eq!(ep.uri.scheme_str(), Some(scheme), "{s:?}: wrong scheme");
    assert!(!ep.host.is_empty(), "{s:?}: accepted with an empty host");
    assert!(ep.path.starts_with('/'), "{s:?}: path {:?}", ep.path);
    let default_port = if ep.secure { 443 } else { 80 };
    assert_eq!(
        ep.port,
        ep.uri.port_u16().unwrap_or(default_port),
        "{s:?}: wrong port"
    );

    let bare_host = ep.host.trim_start_matches('[').trim_end_matches(']');
    if bare_host.parse::<IpAddr>().is_ok() {
        let addr = ep.socket_address();
        assert!(
            addr.parse::<SocketAddr>().is_ok(),
            "{s:?}: socket_address {addr:?} is not a socket address"
        );
    }

    for (uri, expected_scheme) in [
        (ep.request_uri(), scheme),
        (
            ep.http_request_uri(),
            if ep.secure { "https" } else { "http" },
        ),
    ] {
        let uri = uri.unwrap_or_else(|e| panic!("{s:?}: cannot build request URI: {e}"));
        assert_eq!(uri.scheme_str(), Some(expected_scheme), "{s:?}: {uri}");
        assert_eq!(
            uri.authority().map(|a| a.as_str()),
            Some(ep.authority.as_str()),
            "{s:?}: {uri}"
        );
        assert_eq!(uri.path(), ep.path, "{s:?}: {uri}");
        assert_eq!(uri.query(), ep.uri.query(), "{s:?}: {uri}");
    }

    let rebuilt = ep.request_uri().expect("checked above").to_string();
    let again = WebSocketEndpoint::parse(&rebuilt)
        .unwrap_or_else(|e| panic!("{s:?}: request URI {rebuilt:?} does not parse back: {e}"));
    assert_eq!(again.secure, ep.secure, "{s:?} -> {rebuilt:?}");
    assert_eq!(again.host, ep.host, "{s:?} -> {rebuilt:?}");
    assert_eq!(again.port, ep.port, "{s:?} -> {rebuilt:?}");
    assert_eq!(again.path, ep.path, "{s:?} -> {rebuilt:?}");
});
