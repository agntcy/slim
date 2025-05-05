// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agp_datapath::{
    messages::{Agent, AgentType},
    pubsub::proto::pubsub::v1::Message,
};
use agp_gw::config::ConfigResult;
use agp_service::{FireAndForgetConfiguration, session, session::SessionConfig};
use rmcp::{
    RoleClient,
    model::{ClientNotification, ClientRequest, ClientResult, JsonRpcMessage, JsonRpcRequest},
    transport::{IntoTransport, SseTransport, sse::SseTransportError},
};

use futures_util::{StreamExt, sink::SinkExt};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace};

struct Session {
    // name of the agent connect to this session
    agw_session: session::Info,
    // send messages to proxy
    tx_proxy: mpsc::Sender<(session::Info, Vec<u8>)>,
}

impl Session {
    fn new(agw_session: session::Info, tx_proxy: mpsc::Sender<(session::Info, Vec<u8>)>) -> Self {
        Session {
            agw_session,
            tx_proxy,
        }
    }

    async fn run_session(&self, mcp_server: String) -> mpsc::Sender<Message> {
        let (tx_session, mut rx_session) = mpsc::channel::<Message>(128);
        let tx_channel = self.tx_proxy.clone();
        let agw_session = self.agw_session.clone();

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

            loop {
                tokio::select! {
                    next_from_gw = rx_session.recv() => {
                        match next_from_gw {
                            None => {
                                debug!("end of the stream from GW, stop receiving loop");
                                break;
                            }
                            Some(msg) => {
                                debug!("received message from GW");
                                // received a message from the GW, send it to the MCP server
                                let payload = msg.get_payload().unwrap().blob.to_vec();
                                let jsonrpcmsg: JsonRpcMessage<ClientRequest, ClientResult, ClientNotification> =
                                    match serde_json::from_slice(&payload) {
                                        Ok(jsonrpcmsg) => jsonrpcmsg,
                                        Err(e) => {
                                            error!("error parsing the message: {}", e.to_string());
                                            continue;
                                        }
                                };
                                debug!("send message {:?}", jsonrpcmsg);
                                sink.send(jsonrpcmsg).await.unwrap();
                            },
                        }
                    }
                    next_from_mpc = stream.next() => {
                        match next_from_mpc {
                            None => {
                                debug!("end of the stream from MCP, stop receiving loop");
                                break;
                            }
                            Some(msg) => {
                                // received a message from the MCP server, send it to the GW
                                let vec = serde_json::to_vec(&msg).unwrap();
                                let _ = tx_channel.send((agw_session.clone(), vec)).await;
                            }
                        }
                    }
                }
            }
        });
        tx_session
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct SessionId {
    /// name of the source of the packet
    source: Agent,
    /// AGP session id
    id: u32,
}

pub struct Proxy {
    name: Agent,
    config: ConfigResult,
    svc_id: agp_config::component::id::ID,
    mcp_server: String,
    connections: HashMap<SessionId, mpsc::Sender<Message>>,
}

impl Proxy {
    pub fn new(
        name: AgentType,
        id: Option<u64>,
        config: ConfigResult,
        svc_id: agp_config::component::id::ID,
        mcp_server: String,
    ) -> Self {
        let agent_id = match id {
            None => rand::random::<u64>(),
            Some(id) => id,
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
        let svc = self.config.services.get_mut(&self.svc_id).unwrap();

        let mut gw_rx = svc
            .create_agent(&self.name)
            .await
            .expect("failed to create agent");

        // run the service - this will create all the connections provided via the config file.
        svc.run().await.unwrap();

        // get the connection id
        let conn_id = svc
            .get_connection_id(&svc.config().clients()[0].endpoint)
            .unwrap();

        // subscribe for local name
        match svc
            .subscribe(
                &self.name,
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

        let res = svc
            .create_session(
                &self.name,
                SessionConfig::FireAndForget(FireAndForgetConfiguration {}),
            )
            .await;
        if res.is_err() {
            panic!("error creating fire and forget session");
        }

        let (proxy_tx, mut proxy_rx) = mpsc::channel(128);

        info!("waiting for incoming messages");
        loop {
            tokio::select! {
                next_from_gw = gw_rx.recv() => {
                    match next_from_gw {
                        None => {
                            debug!("end of the stream, stop the loop");
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

                                        if request.request.method != "initialize" {
                                            error!("received unexpected initialization method {}", request.request.method);
                                            continue;
                                        }

                                        let session = Session::new(msg.info.clone(), proxy_tx.clone());
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
                            debug!("end of the stream, ignore");
                        }
                        Some((info, msg)) => {
                            let src = match info.message_source {
                                Some(ref s) => s.clone(),
                                None => {
                                    error!("cannot send reply message, unkwon destination");
                                    continue;
                                }
                            };
                            let conn_id = match info.input_connection {
                                Some(c) => c,
                                None => {
                                    error!("cannot send reply message, unkwon incoming connection");
                                    continue;
                                }
                            };
                            match svc.publish_to(
                                &self.name,
                                info,
                                src.agent_type(),
                                Some(src.agent_id()),
                                conn_id,
                                msg,
                            ).await {
                                Ok(()) => {
                                    trace!("sent message to destination {:?}", src);
                                }
                                Err(e) => {
                                    error!("error while sending message to agent {:?}: {}", src, e.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
