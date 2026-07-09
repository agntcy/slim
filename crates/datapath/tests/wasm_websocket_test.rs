// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-runtime tests for the wasm WebSocket transport loops
//! (`websocket/stream_wasm.rs`).
//!
//! These exercise the pieces that only exist on `wasm32-unknown-unknown` and
//! that no other test touches: protobuf framing (encode on send / decode on
//! receive), the inbound/outbound channel plumbing produced by
//! [`spawn_transport_tasks`], and cancellation coordination between the two
//! `spawn_local` loops.
//!
//! ## Running
//!
//! Opening a real `gloo_net` `WebSocket` needs a JS `WebSocket` runtime and a
//! reachable echo server that echoes **binary** frames back verbatim. Both are
//! provided out of band via the `WS_ECHO_SERVER_URL` build-time env var (same
//! convention gloo-net uses for its own suite), e.g.:
//!
//! ```sh
//! WS_ECHO_SERVER_URL="ws://127.0.0.1:9001" \
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   cargo test -p agntcy-slim-datapath --target wasm32-unknown-unknown \
//!     --test wasm_websocket_test
//! ```
//!
//! These run in a real browser (`run_in_browser`) because `gloo_net`'s
//! `WebSocket` binds DOM APIs (`web_sys::WebSocket`, `MessageEvent`,
//! `CloseEvent`, `addEventListener`) that Node runners do not provide. When
//! `WS_ECHO_SERVER_URL` is unset every test **skips**, so the file still
//! compiles and links on wasm, guarding the transport code against bitrot; it
//! asserts for real once an echo endpoint is wired up (see `task
//! test:wasm:browser` and the CI `wasm-browser-test` job).
#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use futures::StreamExt;
use gloo_net::websocket::futures::WebSocket;
use tokio_util::sync::CancellationToken;

use slim_datapath::api::{ProtoMessage as Message, ProtoName as Name};
use slim_datapath::websocket::spawn_transport_tasks;

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// Echo endpoint that reflects binary frames verbatim. Resolved at compile time
/// so the test is a no-op unless the runner was built with it set.
fn echo_server_url() -> Option<&'static str> {
    option_env!("WS_ECHO_SERVER_URL")
}

/// Bound every socket await so a misbehaving server or runtime fails the test
/// instead of hanging the wasm runner.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// A representative inter-node publish message used to check that a frame
/// survives the encode → WebSocket → echo → decode round trip byte-for-byte.
fn sample_message() -> Message {
    let source = Name::from_strings(["org", "default", "a"]).with_id(1);
    let destination = Name::from_strings(["org", "default", "b"]).with_id(2);
    Message::builder()
        .source(source)
        .destination(destination)
        .application_payload("text/plain", b"hello-wasm-websocket".to_vec())
        .build_publish()
        .expect("valid publish message")
}

async fn open_echo_socket(url: &str) -> WebSocket {
    // `WebSocket::open` returns while the socket is still CONNECTING; the
    // transport loops (and gloo-net's `Sink::poll_ready`) handle the wait.
    WebSocket::open(url).expect("open websocket to echo server")
}

// A protobuf `Message` sent on the outbound channel is encoded, framed onto the
// socket, echoed back, decoded by the read loop, and surfaces on the inbound
// stream byte-for-byte identical. This is the core framing path of
// `stream_wasm.rs`, previously untested on any target.
#[wasm_bindgen_test]
async fn spawn_transport_tasks_round_trips_protobuf_frame() {
    let Some(url) = echo_server_url() else {
        // No echo server provisioned (default CI): nothing to exercise.
        return;
    };

    let websocket = open_echo_socket(url).await;
    let cancel = CancellationToken::new();
    let streams = spawn_transport_tasks(websocket, cancel.clone());

    let sent = sample_message();
    streams
        .outbound
        .send(sent.clone())
        .await
        .expect("outbound channel accepts the message");

    // Some echo servers greet with an informational text frame first; the read
    // loop drops non-binary frames, and a stray binary frame would decode to a
    // different message, so read until our frame comes back (or time out).
    let mut inbound = streams.inbound;
    let received = loop {
        let next = tokio::time::timeout(IO_TIMEOUT, inbound.next())
            .await
            .expect("echo did not arrive before timeout");
        match next {
            Some(Ok(msg)) if msg == sent => break msg,
            Some(Ok(_)) => continue,
            Some(Err(status)) => panic!("inbound stream error: {status}"),
            None => panic!("inbound stream closed before echo arrived"),
        }
    };
    assert_eq!(received, sent, "echoed frame must round-trip unchanged");

    cancel.cancel();
}

// Cancelling the shared token stops both loops: the read loop ends the inbound
// stream, and the write loop drops its receiver so further sends fail. This
// pins the cancellation coordination in `stream_wasm.rs`.
#[wasm_bindgen_test]
async fn spawn_transport_tasks_cancellation_stops_loops() {
    let Some(url) = echo_server_url() else {
        return;
    };

    let websocket = open_echo_socket(url).await;
    let cancel = CancellationToken::new();
    let streams = spawn_transport_tasks(websocket, cancel.clone());
    let mut inbound = streams.inbound;

    cancel.cancel();

    // The read loop observes the cancellation and closes the inbound stream.
    let closed = tokio::time::timeout(IO_TIMEOUT, inbound.next())
        .await
        .expect("inbound stream did not close after cancellation");
    assert!(
        closed.is_none(),
        "inbound stream must terminate once the token is cancelled"
    );

    // The write loop drops its receiver on cancellation, so the outbound sender
    // eventually reports the channel as closed.
    let outbound = streams.outbound;
    let send_failed = tokio::time::timeout(IO_TIMEOUT, async {
        loop {
            if outbound.send(sample_message()).await.is_err() {
                break;
            }
        }
    })
    .await;
    assert!(
        send_failed.is_ok(),
        "outbound sender must fail once the write loop has stopped"
    );
}
