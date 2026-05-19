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

    // Internal token coordinates between the read and write halves: if one
    // exits, the other should tear down too. It must NOT be the external
    // `cancellation_token` — cancelling that one tells the higher-level
    // `process_stream`/`reconnect` to give up, which would defeat reconnect
    // on a natural peer close.
    let internal_cancel = CancellationToken::new();

    let read_external = cancellation_token.clone();
    let read_internal = internal_cancel.clone();
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
                if let Some(ctrl) = owned
                    && let Err(err) = tx.try_send(ctrl)
                {
                    debug!(error = %err, "dropping auto control frame");
                }
                Result::<(), WebSocketError>::Ok(())
            }
        };

        loop {
            tokio::select! {
                _ = read_external.cancelled() => {
                    break;
                }
                _ = read_internal.cancelled() => {
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
                            warn!(
                                "received text websocket frame on binary subprotocol; closing connection with status 1003"
                            );
                            let _ = tx_inbound
                                .send(Err(Status::invalid_argument(
                                    "unexpected text websocket frame on binary subprotocol",
                                )))
                                .await;
                            let _ = control_tx
                                .send(ControlFrame::Close {
                                    code: 1003,
                                    reason: b"unsupported data: text frame".to_vec(),
                                })
                                .await;
                            break;
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
        read_internal.cancel();
    });

    let write_internal = internal_cancel.clone();
    let write_external = cancellation_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = write_external.cancelled() => {
                    let _ = write_half.write_frame(Frame::close(1000, &[])).await;
                    let _ = write_half.flush().await;
                    break;
                }
                _ = write_internal.cancelled() => {
                    let _ = write_half.write_frame(Frame::close(1000, &[])).await;
                    let _ = write_half.flush().await;
                    break;
                }
                // Prioritise control frames so Pongs / Close echoes are not
                // starved by a heavy outbound binary stream.
                maybe_ctrl = rx_control.recv() => {
                    let Some(ctrl) = maybe_ctrl else {
                        break;
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

        write_internal.cancel();
    });

    WebSocketStreams {
        inbound: ReceiverStream::new(rx_inbound),
        outbound: tx_outbound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use fastwebsockets::{Frame, OpCode, Payload};
    use slim_config::client::{ClientConfig, TransportChannel};
    use slim_config::server::ServerConfig;
    use slim_config::tls::client::TlsClientConfig;
    use slim_config::tls::server::TlsServerConfig;
    use slim_config::transport::TransportProtocol;
    use slim_config::websocket::client::WebSocketClientChannel;
    use slim_config::websocket::server::OnAcceptedWebSocket;
    use std::net::TcpListener as StdTcpListener;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::TcpStream as TokioTcpStream;
    use tokio_util::sync::CancellationToken;

    fn available_port() -> u16 {
        StdTcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("local_addr")
            .port()
    }

    async fn wait_for_server_ready(addr: &str, max_attempts: u32) -> bool {
        for attempt in 0..max_attempts {
            if TokioTcpStream::connect(addr).await.is_ok() {
                return true;
            }
            let backoff = Duration::from_millis(25 * (1 + attempt as u64).min(10));
            tokio::time::sleep(backoff).await;
        }
        false
    }

    async fn start_server_with_transport_tasks(port: u16) -> CancellationToken {
        let endpoint = format!("ws://127.0.0.1:{port}");
        let server_conf = ServerConfig::with_endpoint(&endpoint)
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());

        let cancel = CancellationToken::new();
        let cancel_for_cb = cancel.clone();

        let on_accepted: OnAcceptedWebSocket = Arc::new(move |conn| {
            let cancel = cancel_for_cb.clone();
            Box::pin(async move {
                // Drives the read/write tasks (and thus auto_pong/auto_close).
                let _streams = spawn_transport_tasks(conn.websocket, cancel);
                // Keep the streams alive for the test duration.
                std::mem::forget(_streams);
            })
        });

        let (signal, watch) = drain::channel();
        // Keep the signal alive for the lifetime of the test; dropping it
        // would immediately drain the server.
        std::mem::forget(signal);
        let token = server_conf
            .run_websocket_server(watch, on_accepted)
            .await
            .expect("server start");

        assert!(
            wait_for_server_ready(&format!("127.0.0.1:{port}"), 40).await,
            "server not ready in time"
        );

        token
    }

    async fn connect_ws_client(port: u16) -> WebSocketClientChannel {
        let endpoint = format!("ws://127.0.0.1:{port}");
        let cfg = ClientConfig::with_endpoint(&endpoint)
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure());
        match cfg.to_channel().await.expect("connect") {
            TransportChannel::Websocket(ws) => *ws,
            TransportChannel::Grpc(_) => panic!("expected websocket"),
        }
    }

    #[tokio::test]
    async fn test_websocket_ping_pong_exchange() {
        let port = available_port();
        let server_token = start_server_with_transport_tasks(port).await;

        let mut channel = connect_ws_client(port).await;

        channel
            .websocket
            .write_frame(Frame::new(
                true,
                OpCode::Ping,
                None,
                Payload::Borrowed(b"hi"),
            ))
            .await
            .expect("write ping");

        let payload = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let frame = channel.websocket.read_frame().await.expect("read frame");
                if frame.opcode == OpCode::Pong {
                    return frame.payload.to_vec();
                }
                if frame.opcode == OpCode::Close {
                    panic!("server closed before sending pong");
                }
            }
        })
        .await
        .expect("did not receive pong in time");

        assert_eq!(payload, b"hi");

        server_token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_close_frame_echo() {
        let port = available_port();
        let server_token = start_server_with_transport_tasks(port).await;

        let mut channel = connect_ws_client(port).await;

        channel
            .websocket
            .write_frame(Frame::close(1000, b"bye"))
            .await
            .expect("write close");

        let saw_close_or_error = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match channel.websocket.read_frame().await {
                    Ok(frame) => {
                        if frame.opcode == OpCode::Close {
                            return true;
                        }
                    }
                    Err(_) => return true,
                }
            }
        })
        .await
        .unwrap_or(false);

        assert!(saw_close_or_error, "server did not echo close in time");

        server_token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_multiple_sequential_connections() {
        let port = available_port();
        let server_token = start_server_with_transport_tasks(port).await;

        for i in 0..3 {
            let _ = tokio::time::timeout(Duration::from_secs(5), connect_ws_client(port))
                .await
                .unwrap_or_else(|_| panic!("connection #{} timed out", i));
        }

        server_token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_cancellation_blocks_new_clients() {
        let port = available_port();
        let server_token = start_server_with_transport_tasks(port).await;

        // Sanity-check: server is up.
        let _ = connect_ws_client(port).await;

        server_token.cancel();

        // Wait until listener is actually gone.
        for _ in 0..40 {
            if TokioTcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let endpoint = format!("ws://127.0.0.1:{port}");
        let cfg = ClientConfig::with_endpoint(&endpoint)
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure());
        assert!(
            cfg.to_channel().await.is_err(),
            "should not connect after server cancellation"
        );
    }
}
