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
use rmcp::service::RunningService;
use rmcp::RoleClient;
use rmcp::model::InitializeRequestParam;
use rmcp::model::JsonRpcRequest;
use rmcp::model::ServerJsonRpcMessage;
use serde_json::Value;
use rmcp::model::NumberOrString::Number;
use rmcp::model::ServerResult;
use rmcp::model::JsonRpcMessage;
use rmcp::model::ClientNotification;
use rmcp::model::ClientRequest;
use agp_service::Service;
use agp_datapath::messages::AgentType;
use agp_gw::config::ConfigResult;
use agp_service::session;

pub struct Proxy {
    name: Agent,
    config: ConfigResult,
    svc_id: agp_config::component::id::ID,
    connections: HashMap<Agent, RunningService<RoleClient, InitializeRequestParam>> 
}

impl Proxy {
    pub async fn new(name: AgentType, id: Option<u64>, config: ConfigResult, svc_id: agp_config::component::id::ID) -> Self {
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
            connections: HashMap::new(),
        }
    }

    pub async fn start(&mut self) {
        // create service from config
        let mut svc = self.config.services.get_mut(&self.svc_id).unwrap();

        let mut rx = svc
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

        info!("waiting for incoming messages");
        loop {
            match rx.recv().await {
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
                                // TODO spwan a new thread
                                info!("the source {} those not exists, create a new connection", src);
                                // get parameters to setup the connection
                                let payload = msg.message.get_payload().unwrap().blob.to_vec();
                                let str = String::from_utf8(payload.clone()).unwrap();
                                info!("str {:?}", str);

                                let request: JsonRpcRequest = serde_json::from_slice(&payload).unwrap();

                                info!("{:?}", request);

                                if request.request.method != "initialize" { // TODO check this if you can do better
                                    error!("received unexpected initalizatio method {}", request.request.method);
                                }

                                let params = serde_json::from_value::<InitializeRequestParam>(serde_json::Value::Object(request.request.params)).unwrap();
                                info!("Version {:?}", params.protocol_version);
                                info!("Capabilities {:?}", params.capabilities);
                                info!("client info {:?}", params.client_info);

                                // TODO get MPC address from command line
                                let transport = SseTransport::start("http://localhost:8000/sse").await.unwrap();
                                let client_info = ClientInfo {
                                    protocol_version: params.protocol_version,
                                    capabilities: params.capabilities,
                                    client_info: params.client_info,
                                };
                                let client = client_info.serve(transport).await.unwrap();

                                let server_info = client.peer_info();
                                info!("Connected to server: {server_info:#?}");

                                // reply
                                let reply = ServerJsonRpcMessage::response( 
                                    ServerResult::InitializeResult(server_info.clone()),
                                    Number(0),
                                );
                                info!("reply {:?}", reply);

                                let vec = serde_json::to_vec(&reply).unwrap();

                                send(&mut svc, &self.name, &src, msg.info, conn_id, vec).await;

                                // store the new connection
                                self.connections.insert(src, client);
                            }
                            Some(client) => {
                                info!("connection exists for source {}, forward MCP message", src);

                                let payload = msg.message.get_payload().unwrap().blob.to_vec();
                                let str = String::from_utf8(payload.clone()).unwrap();
                                info!("str {:?}", str);
                            
                                // this should be done in better way
                                let parsed: Value = serde_json::from_slice(&payload).unwrap();
                                info!("parsed {:?}", parsed);
                                let method = parsed.get("method").unwrap().to_string();
                                info!("method {:?}", method);

                                if method.contains("notifications") {
                                    info!("received a notification");
                                    let not: ClientNotification = serde_json::from_slice(&payload).unwrap();
                                    client.send_notification(not).await.unwrap();
                                } else {
                                    // TODO: check https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/service.rs#L674
                                    info!("received a request");
                                    let req: ClientRequest = serde_json::from_slice(&payload).unwrap();
                                    let reply = client.send_request(req).await.unwrap();
                                    info!("reply {:?}", reply);
                                    let reply_map = serde_json::to_value(reply).unwrap();
                                    // TODO number cannot be hardcoded
                                    let to_send: JsonRpcMessage = JsonRpcMessage::response(reply_map.as_object().unwrap().clone(), Number(1));
                                    let vec = serde_json::to_vec(&to_send).unwrap();
                                    send(&mut svc, &self.name, &src, msg.info, conn_id, vec).await;  
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("an error occured while receiving a message {:?}", e);
                    }
                }
            }
        }
    }
}

async fn send(svc: &mut Service, source: &Agent, destination: &Agent, session_info: session::Info, conn_id: u64, msg: Vec<u8>) {
    match svc.publish_to(
        source,
        session_info,
        destination.agent_type(),
        Some(destination.agent_id()),
        conn_id,
        msg,
    ).await {
        Ok(()) => {
            trace!("message to destionation {}", destination);
        }
        Err(e) => {
            error!("error while sending message to agent {}: {}", destination, e);
        }
    }
}
