// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-side websocket transport tasks.
//!
//! Mirrors `websocket/stream.rs` but uses [`gloo_net::websocket::futures::WebSocket`]
//! and `wasm_bindgen_futures::spawn_local` so the data plane's
//! `MessageProcessor::try_to_connect_websocket` can drive a websocket
//! connection from `wasm32-unknown-unknown` builds.

use futures::sink::SinkExt;
use futures::stream::StreamExt;
use prost::Message as ProstMessage;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use crate::Status;
use crate::api::proto::dataplane::v1::Message;
use crate::runtime::CancellationToken;

pub(crate) struct WebSocketStreams {
    pub(crate) inbound: ReceiverStream<Result<Message, Status>>,
    pub(crate) outbound: mpsc::Sender<Message>,
}

/// Spawn read/write loops over a browser `WebSocket`. The returned channels
/// look identical to the native flavor in `stream.rs`, so
/// [`crate::message_processing::MessageProcessor`] can plug them into its
/// [`crate::connection::Connection`] and `process_stream` machinery without
/// caring which transport produced them.
pub(crate) fn spawn_transport_tasks(
    websocket: gloo_net::websocket::futures::WebSocket,
    cancellation_token: CancellationToken,
) -> WebSocketStreams {
    let (ws_sink, ws_stream) = websocket.split();

    let (tx_inbound, rx_inbound) = mpsc::channel::<Result<Message, Status>>(128);
    let (tx_outbound, rx_outbound) = mpsc::channel::<Message>(128);

    let read_cancel = cancellation_token.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut stream = ws_stream;
        loop {
            let next = tokio::select! {
                _ = read_cancel.cancelled() => break,
                next = stream.next() => next,
            };

            match next {
                None => {
                    debug!("websocket read loop closed by peer");
                    break;
                }
                Some(Ok(gloo_net::websocket::Message::Bytes(bytes))) => {
                    match Message::decode(&bytes[..]) {
                        Ok(msg) => {
                            if tx_inbound.send(Ok(msg)).await.is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            let _ = tx_inbound
                                .send(Err(Status::invalid_argument(format!(
                                    "invalid protobuf payload in websocket frame: {err}",
                                ))))
                                .await;
                        }
                    }
                }
                Some(Ok(gloo_net::websocket::Message::Text(_))) => {
                    warn!("ignoring text websocket frame, expected binary protobuf frame");
                }
                Some(Err(err)) => {
                    let _ = tx_inbound
                        .send(Err(Status::unavailable(format!(
                            "websocket read error: {err}",
                        ))))
                        .await;
                    break;
                }
            }
        }
        read_cancel.cancel();
    });

    let write_cancel = cancellation_token.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut sink = ws_sink;
        let mut rx = rx_outbound;
        loop {
            let next = tokio::select! {
                _ = write_cancel.cancelled() => None,
                next = rx.recv() => next,
            };

            let msg = match next {
                Some(msg) => msg,
                None => break,
            };

            let payload = msg.encode_to_vec();
            if let Err(err) = sink
                .send(gloo_net::websocket::Message::Bytes(payload))
                .await
            {
                warn!(error = %err, "websocket write error");
                break;
            }
        }
        write_cancel.cancel();
    });

    WebSocketStreams {
        inbound: ReceiverStream::new(rx_inbound),
        outbound: tx_outbound,
    }
}
