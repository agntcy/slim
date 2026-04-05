// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use fastwebsockets::{FragmentCollectorRead, Frame, OpCode, WebSocketError};
use prost::Message as ProstMessage;
use slim_config::websocket::common::UpgradedWebSocket;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::Status;
use tracing::{debug, warn};

use crate::api::proto::dataplane::v1::Message;

pub(crate) struct WebSocketStreams {
    pub(crate) inbound: ReceiverStream<Result<Message, Status>>,
    pub(crate) outbound: mpsc::Sender<Message>,
}

pub(crate) fn spawn_transport_tasks(
    websocket: UpgradedWebSocket,
    cancellation_token: CancellationToken,
) -> WebSocketStreams {
    let (mut read_half, mut write_half) = websocket.split(tokio::io::split);
    read_half.set_auto_close(false);
    read_half.set_auto_pong(false);

    let mut reader = FragmentCollectorRead::new(read_half);

    let (tx_inbound, rx_inbound) = mpsc::channel::<Result<Message, Status>>(128);
    let (tx_outbound, mut rx_outbound) = mpsc::channel::<Message>(128);

    let read_cancel = cancellation_token.clone();
    tokio::spawn(async move {
        let mut noop_send = |_frame: Frame<'_>| async move { Result::<(), WebSocketError>::Ok(()) };

        loop {
            tokio::select! {
                _ = read_cancel.cancelled() => {
                    break;
                }
                frame = reader.read_frame::<_, WebSocketError>(&mut noop_send) => {
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(WebSocketError::ConnectionClosed | WebSocketError::UnexpectedEOF) => {
                            debug!("websocket read loop closed by peer");
                            break;
                        }
                        Err(err) => {
                            let _ = tx_inbound
                                .send(Err(Status::unavailable(format!(
                                    "websocket read error: {err}",
                                ))))
                                .await;
                            break;
                        }
                    };

                    match frame.opcode {
                        OpCode::Binary => match Message::decode(&frame.payload[..]) {
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
                        },
                        OpCode::Close => break,
                        OpCode::Text => {
                            warn!("ignoring text websocket frame, expected binary protobuf frame");
                        }
                        OpCode::Ping | OpCode::Pong | OpCode::Continuation => {
                            // Control and continuation frames are handled by fastwebsockets;
                            // only complete binary payloads are forwarded to datapath processing.
                        }
                    }
                }
            }
        }

        read_cancel.cancel();
    });

    let write_cancel = cancellation_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = write_cancel.cancelled() => {
                    let _ = write_half.write_frame(Frame::close(1000, &[])).await;
                    let _ = write_half.flush().await;
                    break;
                }
                maybe_msg = rx_outbound.recv() => {
                    let msg = match maybe_msg {
                        Some(msg) => msg,
                        None => break,
                    };

                    let payload = msg.encode_to_vec();
                    if let Err(err) = write_half.write_frame(Frame::binary(payload.into())).await {
                        warn!(error = %err, "websocket write error");
                        break;
                    }

                    if let Err(err) = write_half.flush().await {
                        warn!(error = %err, "websocket flush error");
                        break;
                    }
                }
            }
        }

        write_cancel.cancel();
    });

    WebSocketStreams {
        inbound: ReceiverStream::new(rx_inbound),
        outbound: tx_outbound,
    }
}
