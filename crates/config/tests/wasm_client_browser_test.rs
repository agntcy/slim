// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-hosted tests for the wasm WebSocket client channel
//! (`websocket/client_wasm.rs::to_websocket_channel`).
//!
//! Unlike the transport-selection checks in `wasm_client_test.rs` (which run
//! under Node), these open a **real** `gloo_net` `WebSocket`, which needs the
//! browser DOM WebSocket API — hence `run_in_browser`. They also need a
//! reachable endpoint, supplied at build time via `WS_ECHO_SERVER_URL`; when it
//! is unset the tests skip. See `datapath/tests/wasm_websocket_test.rs` for the
//! full running convention (`task test:wasm:browser`).
#![cfg(target_arch = "wasm32")]

use slim_config::client::{ClientConfig, TransportChannel};
use slim_config::transport::TransportProtocol;

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

// `to_channel` on a `ws://` endpoint opens a real browser `WebSocket` and yields
// a WebSocket channel whose socket can be taken exactly once — the take-once
// contract `message_processing` relies on.
#[wasm_bindgen_test]
async fn to_channel_opens_websocket_and_socket_is_taken_once() {
    let Some(url) = option_env!("WS_ECHO_SERVER_URL") else {
        return;
    };

    let cfg = ClientConfig::with_endpoint(url);
    assert_eq!(cfg.resolved_transport(), TransportProtocol::Websocket);

    match cfg.to_channel().await {
        Ok(TransportChannel::Websocket(channel)) => {
            assert!(
                channel.take_websocket().is_some(),
                "first take yields the opened socket"
            );
            assert!(
                channel.take_websocket().is_none(),
                "socket can only be taken once"
            );
        }
        Ok(_) => panic!("a ws:// endpoint must resolve to a websocket channel"),
        Err(err) => panic!("to_channel should open the socket, got: {err:?}"),
    }
}
