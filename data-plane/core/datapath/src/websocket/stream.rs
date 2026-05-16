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

/// Control frame to forward from the read task to the write task so that
/// `set_auto_close(true)` / `set_auto_pong(true)` can actually be honoured.
enum ControlFrame {
    /// Reply to a peer Ping with a Pong carrying the same payload.
    Pong(Vec<u8>),
    /// Echo back a peer Close frame to complete the closing handshake.
    Close { code: u16, reason: Vec<u8> },
}

pub(crate) fn spawn_transport_tasks(
    websocket: UpgradedWebSocket,
    cancellation_token: CancellationToken,
) -> WebSocketStreams {
    let (mut read_half, mut write_half) = websocket.split(tokio::io::split);
    read_half.set_auto_close(true);
    read_half.set_auto_pong(true);

    let mut reader = FragmentCollectorRead::new(read_half);

    let (tx_inbound, rx_inbound) = mpsc::channel::<Result<Message, Status>>(128);
    let (tx_outbound, mut rx_outbound) = mpsc::channel::<Message>(128);
    let (tx_control, mut rx_control) = mpsc::channel::<ControlFrame>(8);

    let read_cancel = cancellation_token.clone();
    let control_tx = tx_control.clone();
    tokio::spawn(async move {
        // fastwebsockets invokes this callback for auto-generated Pong/Close
        let mut send_control = |frame: Frame<'_>| {
            let owned = match frame.opcode {
                OpCode::Pong => Some(ControlFrame::Pong(frame.payload.to_vec())),
                OpCode::Close => {
                    let payload = frame.payload.as_ref();
                    let (code, reason) = if payload.len() >= 2 {
                        let code = u16::from_be_bytes([payload[0], payload[1]]);
                        (code, payload[2..].to_vec())
                    } else {
                        (1000, Vec::new())
                    };
                    Some(ControlFrame::Close { code, reason })
                }
                _ => None,
            };
            let tx = control_tx.clone();
            async move {
                if let Some(ctrl) = owned {
                    let _ = tx.send(ctrl).await;
                }
                Result::<(), WebSocketError>::Ok(())
            }
        };

        loop {
            tokio::select! {
                _ = read_cancel.cancelled() => {
                    break;
                }
                frame = reader.read_frame::<_, WebSocketError>(&mut send_control) => {
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
                            // Pong/Close auto-responses are emitted via send_control above;
                            // continuation frames are reassembled by FragmentCollectorRead.
                        }
                    }
                }
            }
        }

        // Dropping tx_control here lets the write task observe end-of-stream
        // on the control channel if the read task exits first.
        drop(control_tx);
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
                // Prioritise control frames so Pongs / Close echoes are not
                // starved by a heavy outbound binary stream.
                maybe_ctrl = rx_control.recv() => {
                    let Some(ctrl) = maybe_ctrl else {
                        continue;
                    };
                    let (frame, is_close) = match ctrl {
                        ControlFrame::Pong(payload) => (Frame::pong(payload.into()), false),
                        ControlFrame::Close { code, reason } => {
                            (Frame::close(code, &reason), true)
                        }
                    };
                    if let Err(err) = write_half.write_frame(frame).await {
                        warn!(error = %err, "websocket control write error");
                        break;
                    }
                    if let Err(err) = write_half.flush().await {
                        warn!(error = %err, "websocket flush error");
                        break;
                    }
                    if is_close {
                        // We've completed the closing handshake; stop writing.
                        break;
                    }
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
