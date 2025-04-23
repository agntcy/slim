use anyhow::Result;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParam, ClientCapabilities, ClientInfo, Implementation},
    transport::SseTransport,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use agp_gw::config;
use agp_datapath::messages::Agent;
use agp_service::StreamingConfiguration;
use agp_service::session::SessionConfig;
use agp_service::FireAndForgetConfiguration;
use tracing::{info, error};
use std::collections::HashMap;
use rmcp::service::RunningService;
use rmcp::RoleClient;
use rmcp::model::InitializeRequestParam;
use rmcp::model::JsonRpcRequest;
use rmcp::model::InitializeResultMethod;
use rmcp::model::ServerJsonRpcMessage;
use serde_json::Value;
use rmcp::model::NumberOrString::Number;
use rmcp::model::ServerResult;
use rmcp::model::JsonRpcMessage;
use rmcp::model::JsonRpcNotification;
use rmcp::model::NotificationNoParam;
use rmcp::model::Notification;
use rmcp::model::Request;
use rmcp::model::ClientNotification;
use rmcp::model::ClientRequest;

#[tokio::main]
async fn main() {
    // TODO map from Agent to connection
    let mut connections: HashMap<Agent, RunningService<RoleClient, InitializeRequestParam>> = HashMap::new(); 

    // init AGP app
    // TODO get config file form command line
    let mut config = config::load_config("../../../data-plane/config/base/client-config.yaml").expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // create local agent
    // TODO get name from commad line
    // TODO randon id
    let id = 50;
    let agent_name = Agent::from_strings("cisco","mcp", "proxy", id);
    let mut rx = svc
        .create_agent(&agent_name)
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
            &agent_name,
            agent_name.agent_type(),
            agent_name.agent_id_option(),
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
            &agent_name,
            SessionConfig::FireAndForget(FireAndForgetConfiguration {})
        )
        .await;
    if res.is_err() {
        panic!("error creating fire and forget session");
    }

    info!("waiting for incoming messages");
    // wait for messages
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

                    //info!("--GOT MESSAGE: {:?}", msg);
                    //let payload = msg.message.get_payload().unwrap().blob.to_vec();
                    //let val: JsonRpcRequest = serde_json::from_slice(&payload).unwrap();
                    //let string =  String::from_utf8(payload);
                    //info!("---CONTENT {:?}", val);


                    let src = msg.message.get_source();
                    match connections.get(&src) {
                        None => {
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
                            //let result: Value = serde_json::to_value(&server_info).unwrap();
                            //info!("result {:?}", result);
                            let reply = ServerJsonRpcMessage::response( 
                                ServerResult::InitializeResult(server_info.clone()),
                                Number(0),
                            );
                            info!("reply {:?}", reply);

                            let vec = serde_json::to_vec(&reply).unwrap();

                            svc.publish_to(
                                &agent_name,
                                msg.info,
                                src.agent_type(),
                                Some(src.agent_id()),
                                conn_id,
                                vec,
                            ).await.unwrap();

                            // store the new connection
                            connections.insert(src, client);
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
                                let to_send: JsonRpcMessage = JsonRpcMessage::response(reply_map.as_object().unwrap().clone(), Number(1));
                                let vec = serde_json::to_vec(&to_send).unwrap();
                                svc.publish_to(
                                    &agent_name,
                                    msg.info,
                                    src.agent_type(),
                                    Some(src.agent_id()),
                                    conn_id,
                                    vec,
                                ).await.unwrap();    
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