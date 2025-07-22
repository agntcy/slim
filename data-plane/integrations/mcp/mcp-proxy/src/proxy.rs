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
use slim::config::ConfigResult;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::{
    api::ProtoMessage as Message,
    messages::{Agent, AgentType},
};
use slim_service::{
    FireAndForgetConfiguration,
    session::{self, SessionConfig},
    timer::{self, Timer},
};

use futures_util::{StreamExt, sink::SinkExt};
use rmcp::model::NumberOrString::Number;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, time};
use tracing::{debug, error, info, trace};

use async_trait::async_trait;

const PING_INTERVAL: u64 = 20;
const MAX_PENDING_PINGS: usize = 3;

struct PingTimerObserver {
    tx_proxy_session: mpsc::Sender<u32>,
}

#[async_trait]
impl timer::TimerObserver for PingTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for rtx {}, retry", timeouts, timer_id);
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

struct ProxySession {
    // name of the agent connect to this session
    slim_session: session::Info,
    // send messages to proxy
    tx_proxy: mpsc::Sender<Result<(session::Info, Vec<u8>), session::Info>>,
}

impl ProxySession {
    fn new(
        slim_session: session::Info,
        tx_proxy: mpsc::Sender<Result<(session::Info, Vec<u8>), session::Info>>,
    ) -> Self {
        ProxySession {
            slim_session,
            tx_proxy,
        }
    }

    async fn run_session(&self, mcp_server: String) -> mpsc::Sender<Message> {
        let (tx_session, mut rx_session) = mpsc::channel::<Message>(128);
        let tx_channel = self.tx_proxy.clone();
        let slim_session = self.slim_session.clone();

        tokio::spawn(async move {
            // connect to the MCP server
            let transport = match SseTransport::start(mcp_server).await {
                Ok(transport) => transport,
                Err(e) => {
                    error!("error connecting to the MCP server: {}", e.to_string());
                    return;
                }
            };

            // get streams
            let (mut sink, mut stream) =
                <SseTransport as IntoTransport<RoleClient, SseTransportError, ()>>::into_transport(
                    transport,
                );

            // start new ping timer
            let (tx_timer, mut rx_timer) = mpsc::channel(128);
            let ping_timer_observer = Arc::new(PingTimerObserver {
                tx_proxy_session: tx_timer,
            });
            let mut ping_timer = Timer::new(
                1,
                timer::TimerType::Constant,
                Duration::from_secs(PING_INTERVAL),
                None,
                None,
            );
            ping_timer.start(ping_timer_observer);
            let mut pending_pings = HashSet::new();

            loop {
                tokio::select! {
                    next_from_slim = rx_session.recv() => {
                        match next_from_slim {
                            None => {
                                debug!("end of the stream from SLIM, stop receiving loop");
                                ping_timer.stop();
                                let _ = sink.close().await;
                                let _ = tx_channel.send(Err(slim_session.clone())).await;
                                break;
                            }
                            Some(msg) => {
                                debug!("received message from SLIM");
                                // received a message from the SLIM, send it to the MCP server
                                let payload = msg.get_payload().unwrap().blob.to_vec();
                                let jsonrpcmsg: JsonRpcMessage<ClientRequest, ClientResult, ClientNotification> =
                                    match serde_json::from_slice(&payload) {
                                        Ok(jsonrpcmsg) => jsonrpcmsg,
                                        Err(e) => {
                                            error!("error parsing the message: {}", e.to_string());
                                            continue;
                                        }
                                };
                                match jsonrpcmsg {
                                    JsonRpcMessage::Response(json_rpc_response) => {
                                        debug!("received response message: {:?}", json_rpc_response);
                                        // in this case the message may be the response for
                                        // an MCP ping message the the proxy sent to the
                                        // client. In all the other cases we forward the reply
                                        // to the real MCP server.
                                        // ping message format:
                                        // JsonRpcResponse { jsonrpc: JsonRpcVersion2_0, id: Number(1), result: EmptyResult(EmptyObject) }

                                        match json_rpc_response.result {
                                            EmptyResult(_) => {
                                                // this can be a ping response
                                                match json_rpc_response.id {
                                                    Number(index) => {
                                                        if pending_pings.contains(&index) {
                                                            // this is a ping reply, clear all pending pings
                                                            // here we remove all the pending pings because we have the
                                                            // prove that the client is still alive. maybe previous packets got lost
                                                            debug!("received ping response with id  {:?}, clear the pending pings", index);
                                                            pending_pings.clear();
                                                        } else {
                                                            // this index is unknown so it may be something else
                                                            // forward to the server
                                                            debug!("forward message to the server {:?}", json_rpc_response);
                                                            sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.unwrap();
                                                        }
                                                    }
                                                    _ => {
                                                        // not a ping, simply forward
                                                        debug!("forward message to the server {:?}", json_rpc_response);
                                                        sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.unwrap();
                                                    }
                                                }
                                            }
                                            _ => {
                                                // not a ping, simply forward
                                                debug!("forward message to the server {:?}", json_rpc_response);
                                                sink.send(rmcp::model::JsonRpcMessage::Response(json_rpc_response)).await.unwrap();
                                            }
                                        }
                                    }
                                    _ => {
                                        // this is not a message response, simply forward
                                        debug!("forward message to the server {:?}", jsonrpcmsg);
                                        sink.send(jsonrpcmsg).await.unwrap();
                                    },
                                }
                            },
                        }
                    }
                    next_from_mpc = stream.next() => {
                        match next_from_mpc {
                            None => {
                                info!("end of the stream from MCP, stop receiving loop");
                                ping_timer.stop();
                                let _ = sink.close().await;
                                let _ = tx_channel.send(Err(slim_session.clone())).await;
                                break;
                            }
                            Some(msg) => {
                                // received a message from the MCP server, send it to SLIM
                                let vec = serde_json::to_vec(&msg).unwrap();
                                let _ = tx_channel.send(Ok((slim_session.clone(), vec))).await;
                            }
                        }
                    }
                    next_from_timer =  rx_timer.recv() => {
                        match next_from_timer {
                            None => {
                                debug!("end of stream from timer, stop receivieng loop");
                                ping_timer.stop();
                                let _ = sink.close().await;
                                let _ = tx_channel.send(Err(slim_session.clone())).await;
                                break;
                            }
                            Some(_) => {
                                if pending_pings.len() >= MAX_PENDING_PINGS {
                                    // too many pending pings, we consider the client down
                                    debug!("the client is not replying to the ping anymore, drop the connection");
                                    ping_timer.stop();
                                    let _ = sink.close().await;
                                    let _ = tx_channel.send(Err(slim_session.clone())).await;
                                    break;
                                }

                                // time to send a new ping to the client
                                let ping_req = PingRequest {
                                    method: PingRequestMethod,

                                };
                                let index = rand::random::<u32>();
                                pending_pings.insert(index);
                                let req = ServerJsonRpcMessage::Request(JsonRpcRequest {
                                    jsonrpc: rmcp::model::JsonRpcVersion2_0,
                                    id: Number(index),
                                    request: rmcp::model::ServerRequest::PingRequest(ping_req)
                                });

                                let vec = serde_json::to_vec(&req).unwrap();
                                let _ = tx_channel.send(Ok((slim_session.clone(), vec))).await;

                            }
                        }
                    }
                }
            }
        });

        // return tx_session
        tx_session
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct SessionId {
    /// name of the source of the packet
    source: Agent,
    /// SLIM session id
    id: u32,
}

pub struct Proxy {
    name: Agent,
    config: ConfigResult,
    svc_id: slim_config::component::id::ID,
    mcp_server: String,
    connections: HashMap<SessionId, mpsc::Sender<Message>>,
}

impl Proxy {
    pub fn new(
        name: AgentType,
        identity: Option<String>,
        id: Option<u64>,
        config: ConfigResult,
        svc_id: slim_config::component::id::ID,
        mcp_server: String,
    ) -> Self {
        let agent_id = match identity {
            None => match id {
                None => rand::random::<u64>(),
                Some(id) => id,
            },
            Some(i) => Agent::agent_id_from_identity(&i),
        };

        let agent_name = Agent::new(name, agent_id);

        Proxy {
            name: agent_name,
            config,
            svc_id,
            mcp_server,
            connections: HashMap::new(),
        }
    }

    pub async fn start(&mut self) {
        // create service from config
        let mut svc = self.config.services.remove(&self.svc_id).unwrap();

        let (app, mut slim_rx) = svc
            .create_app(
                &self.name,
                SharedSecret::new("id", "secret"),
                SharedSecret::new("id", "secret"),
            )
            .await
            .expect("failed to create agent");

        // run the service - this will create all the connections provided via the config file.
        svc.run().await.unwrap();

        // get the connection id
        let conn_id = svc
            .get_connection_id(&svc.config().clients()[0].endpoint)
            .unwrap();

        // subscribe for local name
        match app
            .subscribe(
                self.name.agent_type(),
                self.name.agent_id_option(),
                Some(conn_id),
            )
            .await
        {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a subscription {}", e);
            }
        }

        let res = app
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                None,
            )
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        let (proxy_tx, mut proxy_rx) = mpsc::channel(128);

        info!("waiting for incoming messages");
        loop {
            tokio::select! {
                next_from_slim = slim_rx.recv() => {
                    match next_from_slim {
                        None => {
                            info!("end of the stream, stop the MCP prefix");
                            break;
                        }
                        Some(result) => match result {
                            Ok(msg) => {
                                if !msg.message.is_publish() {
                                    error!("received unexpected message type");
                                    continue;
                                }

                                let session_id = SessionId {
                                    source: msg.message.get_source(),
                                    id: msg.info.id,
                                };
                                match self.connections.get(&session_id) {
                                    None => {
                                        debug!("the session {:?} does not exists, create a new connection", session_id);

                                        // this must be an InitializeRequest
                                        let payload = msg.message.get_payload().unwrap().blob.to_vec();

                                        let request: JsonRpcRequest = match serde_json::from_slice(&payload) {
                                            Ok(request) => request,
                                            Err(e) => {
                                                error!("error while parsing incoming packet: {}", e.to_string());
                                                continue;
                                            }
                                        };

                                        if request.request.method != "initialize" { // InitializeResultMethod
                                            error!("received unexpected initialization method {}", request.request.method);
                                            continue;
                                        }

                                        info!("start new session {:?}", session_id);
                                        let session = ProxySession::new(msg.info.clone(), proxy_tx.clone());
                                        let session_tx = session.run_session(self.mcp_server.clone()).await;

                                        // send the message to the new created session
                                        let _ = session_tx.send(msg.message).await;

                                        // store the new sessiopm
                                        self.connections.insert(session_id, session_tx);
                                    }
                                    Some(session_tx) => {
                                        debug!("connection exists for session {:?}, forward MCP message", session_id);
                                        let _ = session_tx.send(msg.message).await;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("an error occurred while receiving a message {:?}", e);
                            }
                        }
                    }
                }
                next_from_session = proxy_rx.recv() => {
                    match next_from_session {
                        None => {
                            debug!("some proxy session unexpectedly stopped. ignore it");
                        }
                        Some(result) => match result {
                            Ok((info, msg)) => {
                                let src = match info.message_source {
                                    Some(ref s) => s.clone(),
                                    None => {
                                        error!("cannot send reply message, unknown destination");
                                        continue;
                                    }
                                };
                                let conn_id = match info.input_connection {
                                    Some(c) => c,
                                    None => {
                                        error!("cannot send reply message, unknown incoming connection");
                                        continue;
                                    }
                                };
                                match app.publish_to(
                                    info,
                                    src.agent_type(),
                                    Some(src.agent_id()),
                                    conn_id,
                                    msg,
                                ).await {
                                    Ok(()) => {
                                        debug!("sent message to destination {:?}", src);
                                    }
                                    Err(e) => {
                                        error!("error while sending message to agent {:?}: {}", src, e.to_string());
                                    }
                                }
                            }
                            Err(info) => {
                                let src = match info.message_source {
                                    Some(ref s) => s.clone(),
                                    None => {
                                        error!("cannot stop proxy session, unknown destination");
                                        continue;
                                    }
                                };

                                let session_id = SessionId {
                                    source: src,
                                    id: info.id,
                                };

                                // remove the proxy session if it exists
                                self.connections.remove(&session_id);
                                info!("stop session {:?}", session_id);
                            }
                        }
                    }
                }
                _ = slim_signal::shutdown() => {
                    info!("Received shutdown signal, stop mcp-proxy");
                    break;
                }
            }
        }

        info!("shutting down proxy server");
        self.connections.clear();

        // consume the service and get the drain signal
        let signal = svc.signal();

        match time::timeout(self.config.runtime.drain_timeout(), signal.drain()).await {
            Ok(()) => {}
            Err(_) => panic!("timeout waiting for drain for service"),
        }
    }
}
