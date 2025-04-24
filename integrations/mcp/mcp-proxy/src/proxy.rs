use rmcp::{
    ServiceExt,
    model::ClientInfo,
    transport::SseTransport,
};

use agp_datapath::messages::Agent;
use agp_service::session::SessionConfig;
use agp_service::FireAndForgetConfiguration;
use tracing::{info, error, trace};
use std::collections::HashMap;
use rmcp::model::InitializeRequestParam;
use rmcp::model::JsonRpcRequest;
use rmcp::model::ServerJsonRpcMessage;
use serde_json::Value;
use rmcp::model::ServerResult;
use rmcp::model::JsonRpcMessage;
use rmcp::model::ClientNotification;
use rmcp::model::ClientRequest;
use agp_datapath::messages::AgentType;
use agp_gw::config::ConfigResult;
use agp_service::session;
use tokio::sync::mpsc;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProxyError {
    #[error("Connection Error {0}")]
    ConnectionError(String),
    #[error("MCP Client Creation Error {0}")]
    ClientCreationError(String),
    #[error("Parsion Error {0}")]
    ParsingError(String),
    #[error("Error Sending Message to MCP Server {0}")]
    SendToMCPError(String),
}

struct Session {
    // name of the agent connect to this session
    agw_session: session::Info,
    // send messages to proxy
    tx_proxy: mpsc::Sender<Result<(session::Info, Vec<u8>), ProxyError>>,
}

impl Session {
    fn new (agw_session: session::Info, tx_proxy: mpsc::Sender<Result<(session::Info, Vec<u8>), ProxyError>>) -> Self {
        Session {
            agw_session, tx_proxy,
        }
    }

    async fn run_session(&self, mcp_server: String, request: JsonRpcRequest) -> mpsc::Sender<Result<Message, ProxyError>> {
        let (tx_session, mut rx_session) = mpsc::channel::<Result<Message, ProxyError>>(128);
        let tx_channel = self.tx_proxy.clone();
        let agw_session = self.agw_session.clone();

        tokio::spawn(async move {
            // get paramters from the request
            let params = match serde_json::from_value::<InitializeRequestParam>(serde_json::Value::Object(request.request.params)) {
                Ok(params) => params,
                Err(e) => {
                    error!("error parsing init request parametes: {}", e.to_string());
                    return;
                }
            };

            let req_id = request.id;

            let client_info = ClientInfo {
                protocol_version: params.protocol_version,
                capabilities: params.capabilities,
                client_info: params.client_info,
            };

            info!("client paramters {:?}", client_info);

            // init session
            let transport = match SseTransport::start(mcp_server).await {
                Ok(transport) => transport,
                Err(e) => {
                    let _ = tx_channel.send(Err(ProxyError::ConnectionError(e.to_string()))).await;
                    return;
                } 
            };

            let client = match client_info.serve(transport).await {
                Ok(client) => client,
                Err(e) => {
                    let _ = tx_channel.send(Err(ProxyError::ClientCreationError(e.to_string()))).await;
                    return;
                }
            };

            let server_info = client.peer_info();
            info!("Connected to server: {server_info:#?}");

            // reply
            let reply = ServerJsonRpcMessage::response( 
            ServerResult::InitializeResult(server_info.clone()),
                req_id,
            );
            info!("reply {:?}", reply);

            let vec = serde_json::to_vec(&reply).unwrap();
            let _ = tx_channel.send(Ok((agw_session.clone(), vec))).await;

            loop {
                match rx_session.recv().await {
                    None => {
                        info!("end of the stream, break");
                        break;
                    }
                    Some(result) => match result {
                        Ok(msg) => {
                            // received a message to forward to the MCP server
                            let payload = msg.get_payload().unwrap().blob.to_vec();
                            // this should be done in better way
                            let parsed: Value = match serde_json::from_slice(&payload) {
                                Ok(parsed) => parsed,
                                Err(e) => {
                                    let _ = tx_channel.send(Err(ProxyError::ParsingError(e.to_string()))).await;
                                    continue;
                                }
                            };

                            info!("parsed message {:?}", parsed);

                            let method = match parsed.get("method") {
                                Some(val) => val.to_string(),
                                None => {
                                    let _ = tx_channel.send(Err(ProxyError::ParsingError("missing method".to_string()))).await;
                                    continue;
                                }
                            };

                            info!("method {:?}", method);

                            if method.contains("notifications") {
                                info!("received a notification");
                                let not: ClientNotification = match serde_json::from_slice(&payload) {
                                    Ok(not) => not,
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::ParsingError(e.to_string()))).await;
                                        continue;
                                    }
                                }; 

                                match client.send_notification(not).await {
                                    Ok(()) => {}
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::SendToMCPError(e.to_string()))).await;
                                        continue;
                                    }
                                }
                            } else {
                                info!("received a request");
                                // the req id is always present in a request
                                let req_id =  serde_json::from_value(parsed.get("id").unwrap().clone()).unwrap();
                                info!("req id {:?}", req_id);

                                let request: ClientRequest = match serde_json::from_slice(&payload) {
                                    Ok(request) => request,
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::ParsingError(e.to_string()))).await;
                                        continue;
                                    }
                                };

                                info!("client req {:?}", request);
                                let server_reply = match client.send_request(request).await {
                                    Ok(server_reply) => server_reply,
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::SendToMCPError(e.to_string()))).await;
                                        continue;
                                    }
                                };

                                info!("reply {:?}", server_reply);
                                let parsed_reply = match serde_json::to_value(server_reply) {
                                    Ok(parsed_reply) => parsed_reply,
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::ParsingError(e.to_string()))).await;
                                        continue;
                                    }
                                };
                                let reply: JsonRpcMessage = JsonRpcMessage::response(parsed_reply.as_object().unwrap().clone(), req_id);
                                let reply_bytes = match serde_json::to_vec(&reply) {
                                    Ok(reply_bytes) => reply_bytes,
                                    Err(e) => {
                                        let _ = tx_channel.send(Err(ProxyError::ParsingError(e.to_string()))).await;
                                        continue;
                                    }
                                };
                                let _ = tx_channel.send(Ok((agw_session.clone(), reply_bytes))).await;
                            }
                        }
                        Err(e) => {
                            error!("received error message: {:?}", e);
                        }
                    }
                }
            }
        });
        tx_session
    }
}


pub struct Proxy {
    name: Agent,
    config: ConfigResult,
    svc_id: agp_config::component::id::ID,
    mcp_server: String,
    connections: HashMap<Agent, mpsc::Sender<Result<Message, ProxyError>>>,
}

impl Proxy {
    pub fn new(name: AgentType, id: Option<u64>, config: ConfigResult, svc_id: agp_config::component::id::ID, mcp_server: String) -> Self {
        let agent_id = match id {
            None => {
                rand::random::<u64>()
            }
            Some(id) => id
        };

        let agent_name = Agent::new(name, agent_id);

        Proxy{
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
        ).await
        {
            Ok(_) => {}
            Err(e) => {
                panic!("an error accoured while adding a subscription {}", e);
            }
        }

        let res = svc
            .create_session(
                &self.name,
                SessionConfig::FireAndForget(FireAndForgetConfiguration {})
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
                            info!("end of the stream, break");
                            break;
                        }
                        Some(result) => match result {
                            Ok(msg) => {
                                if !msg.message.is_publish() {
                                    error!("received unexpected message type");
                                    continue;
                                }

                                let src = msg.message.get_source();
                                match self.connections.get(&src) {
                                    None => {
                                        info!("the source {} those not exists, create a new connection", src);

                                        // get parameters to setup the connection
                                        let payload = msg.message.get_payload().unwrap().blob.to_vec();

                                        let request: JsonRpcRequest = match serde_json::from_slice(&payload) {
                                            Ok(request) => request,
                                            Err(e) => {
                                                error!("error while parsing incoming packet: {}", e.to_string());
                                                continue;
                                            }
                                        };

                                        info!("{:?}", request);

                                        if request.request.method != "initialize" { // TODO check this if you can do better
                                            error!("received unexpected initalizatio method {}", request.request.method);
                                            continue;
                                        }

                                        /*let params = match serde_json::from_value::<InitializeRequestParam>(serde_json::Value::Object(request.request.params)) {
                                            Ok(params) => params,
                                            Err(e) => {
                                                error!("error parsing init request parametes: {}", e.to_string());
                                                continue;
                                            }
                                        };
                                        
                                        info!("Version {:?}", params.protocol_version);
                                        info!("Capabilities {:?}", params.capabilities);
                                        info!("client info {:?}", params.client_info);*/

                                        let session = Session::new(msg.info.clone(), proxy_tx.clone());
                                        let session_tx = session.run_session(self.mcp_server.clone(), request).await;
                                        self.connections.insert(src, session_tx);
                                    }
                                    Some(client) => {
                                        info!("connection exists for source {}, forward MCP message", src);
                                        let _ = client.send(Ok(msg.message)).await;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("an error occured while receiving a message {:?}", e);
                            }
                        }
                    }
                }
                next_from_session = proxy_rx.recv() => {
                    match next_from_session {
                        None => {
                            info!("end of the stream, ignore");
                        }
                        Some(result) => match result {
                            Ok((info, msg)) => {
                                let src = match info.message_source {
                                    Some(ref s) => s.clone(),
                                    None => {
                                        error!("cannot send reply message, unkwon destionation");
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
                                        trace!("sent message to destionation {:?}", src);
                                    }
                                    Err(e) => {
                                        error!("error while sending message to agent {:?}: {}", src, e.to_string());
                                    }
                                }
                            }   
                            Err(e) => {
                                error!("an error occured: {}", e.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
}
