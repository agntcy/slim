// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-runtime unit tests for the wasm `ClientConfig` transport selection.
//!
//! The browser build only supports the WebSocket transport (gRPC is native
//! only), so `to_channel` must reject any endpoint that resolves to gRPC before
//! attempting to open a socket. This is wasm-only code and cannot run in the
//! native suite, so it is exercised here under wasm-bindgen-test.
//!
//! These do NOT feed the native coverage report; they exist to confirm the
//! browser client-config logic behaves correctly.
//!
//! Run with `task test:wasm`, or:
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     cargo test -p agntcy-slim-config --target wasm32-unknown-unknown \
//!     --test wasm_client_test
#![cfg(target_arch = "wasm32")]

use slim_config::client::ClientConfig;
use slim_config::errors::ConfigError;
use slim_config::transport::TransportProtocol;

use wasm_bindgen_test::wasm_bindgen_test;

// `TransportChannel` is not `Debug`, so assert on the error arm directly rather
// than via `expect_err`.
async fn assert_grpc_rejected(endpoint: &str) {
    let cfg = ClientConfig::with_endpoint(endpoint);
    assert_eq!(
        cfg.resolved_transport(),
        TransportProtocol::Grpc,
        "{endpoint} should infer gRPC"
    );
    match cfg.to_channel().await {
        Ok(_) => panic!("{endpoint} should be rejected in the browser build"),
        Err(err) => assert!(
            matches!(err, ConfigError::WebSocketClientUnsupportedTransport),
            "unexpected error for {endpoint}: {err:?}"
        ),
    }
}

// An `http://` endpoint resolves to gRPC, which is unavailable in the browser,
// so `to_channel` returns the unsupported-transport error rather than opening a
// socket.
#[wasm_bindgen_test]
async fn to_channel_rejects_http_grpc_endpoint() {
    assert_grpc_rejected("http://localhost:46357").await;
}

// A bare `host:port` authority also infers gRPC and is rejected.
#[wasm_bindgen_test]
async fn to_channel_rejects_bare_authority() {
    assert_grpc_rejected("127.0.0.1:46357").await;
}

// `https://` is likewise gRPC (TLS gRPC) and rejected on wasm.
#[wasm_bindgen_test]
async fn to_channel_rejects_https_grpc_endpoint() {
    assert_grpc_rejected("https://example.com:443").await;
}

// ws:// and wss:// resolve to the WebSocket transport on the wasm build.
#[wasm_bindgen_test]
fn websocket_schemes_resolve_to_websocket_transport() {
    assert_eq!(
        ClientConfig::with_endpoint("ws://localhost:46357").resolved_transport(),
        TransportProtocol::Websocket
    );
    assert_eq!(
        ClientConfig::with_endpoint("wss://example.com/socket").resolved_transport(),
        TransportProtocol::Websocket
    );
}
