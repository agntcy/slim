// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use rmcp::model::ClientResult::EmptyResult;
use rmcp::{
    RoleClient,
    model::{
        ClientNotification, ClientRequest, ClientResult, JsonRpcMessage, JsonRpcRequest,
        PingRequest, PingRequestMethod, ServerJsonRpcMessage,
    },
    transport::{IntoTransport, SseTransport, sse::SseTransportError},
};

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;
use slim_session::{
    context::SessionContext,
    notification::Notification,
    timer::{Timer, TimerObserver, TimerType},
};

use futures_util::{StreamExt, sink::SinkExt};
use rmcp::model::NumberOrString::Number;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace};

use async_trait::async_trait;

const PING_INTERVAL: u64 = 20;
const MAX_PENDING_PINGS: usize = 3;

struct PingTimerObserver {
    tx_proxy_session: mpsc::Sender<u32>,
}

#[async_trait]
impl TimerObserver for PingTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!(n_timeouts = %timeouts, %timer_id, "timeout for rtx, retry");
        let _ = self.tx_proxy_session.send(timer_id).await;
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        panic!("timer on failure, this should never happen");
    }

    async fn on_stop(&self, _timer_id: u32) {
        trace!("timer cancelled");
        // nothing to do
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct SessionId {
    /// name of the source of the packet
    source: Name,
    /// SLIM session id
    id: u32,
}

pub struct Proxy {
    name: Name,
    mcp_server: String,
    // retain mapping for active session ids to help with cleanup / debugging
    connections: HashMap<SessionId, ()>,
}

/// Spawn the async task that bridges a SLIM session with the MCP server.
fn start_proxy_session(ctx: SessionContext, mcp_server: String) {
    let session_id_val = ctx.session_arc().unwrap().id();
    ctx.spawn_receiver(move |mut rx, weak| async move {
        info!(%session_id_val, "Session handler task started");

        let binding = weak.upgrade();
        let remote_name = binding.as_ref().unwrap().dst();

        let mut incoming_conn_id: Option<u64> = None;

        // Connect to MCP server
        let transport = match SseTransport::start(mcp_server).await {
            Ok(t) => t,
            Err(e) => {
                error!("error connecting to the MCP server: {}", e.to_string());
                return;
            }
        };
        let (mut sink, mut stream) = <SseTransport as IntoTransport<RoleClient, SseTransportError, ()>>::into_transport(transport);

        // Ping timer setup
        let (tx_timer, mut rx_timer) = mpsc::channel(128);
        let ping_timer_observer = Arc::new(PingTimerObserver { tx_proxy_session: tx_timer });
        let mut ping_timer = Timer::new(1, TimerType::Constant, Duration::from_secs(PING_INTERVAL), None, None);
        ping_timer.start(ping_timer_observer);
        let mut pending_pings: HashSet<u32> = HashSet::new();

        loop {
            tokio::select! {
                next_from_session = rx.recv() => {
                    match next_from_session {
                        None => {
                            debug!("session channel closed");
                            ping_timer.stop();
                            let _ = sink.close().await;
                            break;
                        }
                        Some(Ok(message)) => {
                            if incoming_conn_id.is_none() {
                                // derive remote routing info from first message
                                incoming_conn_id = Some(message.get_incoming_conn());
                                debug!("Initialized remote routing: name={:?} conn_id={:?}", remote_name, incoming_conn_id);
                            }
                            let payload = match message.get_payload() { Some(p) => p.as_application_payload().unwrap().blob.to_vec(), None => { error!("empty payload"); continue; } };
                            let jsonrpcmsg: JsonRpcMessage<ClientRequest, ClientResult, ClientNotification> = match serde_json::from_slice(&payload) {
                                Ok(v) => v,
                                Err(e) => { error!("error parsing message: {}", e); continue; }
                            };
                            match jsonrpcmsg {
                                JsonRpcMessage::Response(json_rpc_response) => {
                                    debug!("received response message: {:?}", json_rpc_response);
                                    match json_rpc_response.result {
                                        EmptyResult(_) => {
                                            match json_rpc_response.id {
                                                Number(index) => {
                                                    if pending_pings.contains(&index) {
                                                        debug!("received ping response id {:?}, clearing pending pings", index);
                                                        pending_pings.clear();
                                                    } else {
                                                        debug!("forward response to MCP server {:?}", json_rpc_response);
                                                        if sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.is_err() { error!("failed sending response to MCP server"); }
                                                    }
                                                }
                                                _ => {
                                                    debug!("forward response to MCP server {:?}", json_rpc_response);
                                                    if sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.is_err() { error!("failed sending response to MCP server"); }
                                                }
                                            }
                                        }
                                        _ => {
                                            debug!("forward response to MCP server {:?}", json_rpc_response);
                                            if sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.is_err() { error!("failed sending response to MCP server"); }
                                        }
                                    }
                                }
                                _ => {
                                    debug!("forward message to MCP server {:?}", jsonrpcmsg);
                                    if sink.send(jsonrpcmsg).await.is_err() { error!("failed forwarding message to MCP server"); }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!("error receiving session message: {:?}", e);
                            ping_timer.stop();
                            let _ = sink.close().await;
                            break;
                        }
                    }
                }
                next_from_mcp = stream.next() => {
                    match next_from_mcp {
                        None => {
                            info!("end of MCP stream");
                            ping_timer.stop();
                            let _ = sink.close().await;
                            break;
                        }
                        Some(msg) => {
                            if let Some(conn) = incoming_conn_id {
                                if let Some(session_arc) = weak.upgrade() {
                                    let vec = serde_json::to_vec(&msg).unwrap();
                                    if let Err(e) = session_arc.publish_to(remote_name, conn, vec, None, None).await { error!("error sending MCP->client message: {}", e); }
                                } else { debug!("session dropped before sending MCP message"); break; }
                            } else {
                                debug!("dropping MCP message: remote not initialized yet");
                            }
                        }
                    }
                }
                timer_ping = rx_timer.recv() => {
                    match timer_ping {
                        None => { debug!("timer channel closed"); break; }
                        Some(_) => {
                            if pending_pings.len() >= MAX_PENDING_PINGS {
                                debug!("client not replying to pings, closing");
                                ping_timer.stop();
                                let _ = sink.close().await;
                                break;
                            }
                            if let Some(conn) = incoming_conn_id && let Some(session_arc) = weak.upgrade() {
                                let ping_req = PingRequest { method: PingRequestMethod };
                                let index = rand::random::<u32>();
                                pending_pings.insert(index);
                                let req = ServerJsonRpcMessage::Request(JsonRpcRequest { jsonrpc: rmcp::model::JsonRpcVersion2_0, id: Number(index), request: rmcp::model::ServerRequest::PingRequest(ping_req) });
                                let vec = serde_json::to_vec(&req).unwrap();
                                if let Err(e) = session_arc.publish_to(remote_name, conn, vec, None, None).await { error!("error sending ping: {}", e); }
                            }
                        }
                    }
                }
            }
        }
        info!("Session handler task ended (session id={})", session_id_val);
    });
}

impl Proxy {
    pub fn new(name: Name, mcp_server: String) -> Self {
        Self {
            name,
            mcp_server,
            connections: HashMap::new(),
        }
    }

    pub async fn start(
        &mut self,
        service: slim_service::Service,
        _drain_timeout: std::time::Duration,
    ) {
        const SECRET: &str = "tUDNjNmc4s6om6yziR4nmBVKKTFCXhfJEiP";

        let (app, mut slim_rx) = service
            .create_app(
                &self.name,
                SharedSecret::new("id", SECRET).expect("Failed to create SharedSecret"),
                SharedSecret::new("id", SECRET).expect("Failed to create SharedSecret"),
            )
            .expect("failed to create app");

        // run the service - this will create all the connections provided via the config file.
        service.run().await.unwrap();

        // get the connection id
        let conn_id = service
            .get_connection_id(&service.config().dataplane_clients()[0].endpoint)
            .unwrap();

        // subscribe for local name
        match app.subscribe(&self.name, Some(conn_id)).await {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a subscription {}", e);
            }
        }

        info!("waiting for incoming messages");
        loop {
            tokio::select! {
                next_from_slim = slim_rx.recv() => {
                    match next_from_slim {
                        None => {
                            info!("end of the stream, stop the MCP prefix");
                            break;
                        }
                        Some(notification) => {
                            match notification {
                                Ok(Notification::NewSession(ctx)) => {
                                    let session = ctx.session_arc().unwrap();
                                    let session_id_val = session.id();
                                    let source_name = session.source().clone();
                                    let session_key = SessionId { source: source_name, id: session_id_val };
                                    self.connections.insert(session_key, ());
                                    start_proxy_session(ctx, self.mcp_server.clone());
                                }
                                Ok(Notification::NewMessage(msg)) => {
                                    // Unexpected standalone app-level message for proxy use-case
                                    debug!("received unexpected NewMessage at app level: {:?}", msg.get_session_header().get_session_id());
                                }
                                Err(e) => {
                                    error!("error receiving notification: {:?}", e);
                                }
                            }
                        }
                    }
                }
                // shutdown signal
                _ = slim_signal::shutdown() => {
                    info!("Received shutdown signal, stop mcp-proxy");
                    break;
                }
            }
        }

        info!("shutting down proxy server");
        self.connections.clear();

        service.shutdown().await.unwrap();
    }
}
