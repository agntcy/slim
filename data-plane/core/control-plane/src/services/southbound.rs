// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    Ack, ControlMessage, control_message::Payload, controller_service_server::ControllerService,
};
use crate::db::{ConnectionDetails, Node, SharedDb};
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus};
use crate::services::group::GroupService;
use crate::services::routes::{ALL_NODES_ID, Route, RouteService};

const REGISTER_TIMEOUT_SECS: u64 = 15;

pub struct SouthboundApiService {
    db: SharedDb,
    cmd_handler: Arc<DefaultNodeCommandHandler>,
    route_service: Arc<RouteService>,
    group_service: Arc<GroupService>,
}

impl SouthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: Arc<DefaultNodeCommandHandler>,
        route_service: Arc<RouteService>,
        group_service: Arc<GroupService>,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
            group_service,
        }
    }
}

#[tonic::async_trait]
impl ControllerService for SouthboundApiService {
    type OpenControlChannelStream = UnboundedReceiverStream<Result<ControlMessage, Status>>;

    async fn open_control_channel(
        &self,
        request: Request<Streaming<ControlMessage>>,
    ) -> Result<Response<Self::OpenControlChannelStream>, Status> {
        let peer_addr = request
            .remote_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();

        let mut stream = request.into_inner();

        let (tx, rx) = mpsc::unbounded_channel::<Result<ControlMessage, Status>>();

        // Send initial ACK to acknowledge the connection.
        let init_ack = ControlMessage {
            message_id: Uuid::new_v4().to_string(),
            payload: Some(Payload::Ack(Ack {
                success: true,
                ..Default::default()
            })),
        };
        tx.send(Ok(init_ack)).ok();

        let db = self.db.clone();
        let cmd_handler = self.cmd_handler.clone();
        let route_service = self.route_service.clone();
        let group_service = self.group_service.clone();

        tokio::spawn(async move {
            let registered_node_id = match receive_register(
                &mut stream,
                &peer_addr,
                &db,
                &cmd_handler,
                &route_service,
                &tx,
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("southbound: registration failed: {e}");
                    return;
                }
            };

            if registered_node_id.is_empty() {
                tracing::info!("southbound: no register request received, closing stream");
                return;
            }

            handle_node_messages(
                stream,
                &registered_node_id,
                &db,
                &cmd_handler,
                &route_service,
                &group_service,
                &tx,
            )
            .await;
        });

        Ok(Response::new(UnboundedReceiverStream::new(rx)))
    }
}

/// Wait for the RegisterNodeRequest with a timeout, save the node, register
/// the stream, and return the node ID. Returns empty string if no register
/// message arrived.
async fn receive_register(
    stream: &mut Streaming<ControlMessage>,
    peer_host: &str,
    db: &SharedDb,
    cmd_handler: &Arc<DefaultNodeCommandHandler>,
    route_service: &Arc<RouteService>,
    tx: &mpsc::UnboundedSender<Result<ControlMessage, Status>>,
) -> Result<String, String> {
    let msg = tokio::time::timeout(Duration::from_secs(REGISTER_TIMEOUT_SECS), stream.message())
        .await
        .map_err(|_| "timeout waiting for register node request".to_string())?
        .map_err(|e| format!("stream error: {e}"))?;

    let msg = match msg {
        Some(m) => m,
        None => return Ok(String::new()),
    };

    let reg_req = match msg.payload {
        Some(Payload::RegisterNodeRequest(r)) => r,
        _ => return Ok(String::new()),
    };

    let mut node_id = reg_req.node_id.clone();
    if let Some(ref group) = reg_req.group_name
        && !group.is_empty()
    {
        node_id = format!("{group}/{node_id}");
    }
    tracing::info!("southbound: registering node {node_id}");

    let conn_details: Vec<ConnectionDetails> = reg_req
        .connection_details
        .iter()
        .map(|d| parse_conn_details(peer_host, d))
        .collect();

    let (_, conn_details_updated) = db
        .save_node(Node {
            id: node_id.clone(),
            group_name: reg_req.group_name.clone(),
            conn_details,
            last_updated: std::time::SystemTime::now(),
        })
        .await
        .map_err(|e| format!("failed to save node: {e}"))?;

    cmd_handler.add_stream(&node_id, tx.clone()).await;
    cmd_handler
        .update_connection_status(&node_id, NodeStatus::Connected)
        .await;

    // Acknowledge the registration.
    let ack = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::Ack(Ack {
            success: true,
            messages: vec!["Node registered successfully".to_string()],
            ..Default::default()
        })),
    };
    tx.send(Ok(ack)).ok();

    route_service
        .node_registered(&node_id, conn_details_updated)
        .await;

    Ok(node_id)
}

/// Main message loop after the node has been registered.
async fn handle_node_messages(
    mut stream: Streaming<ControlMessage>,
    node_id: &str,
    _db: &SharedDb,
    cmd_handler: &Arc<DefaultNodeCommandHandler>,
    route_service: &Arc<RouteService>,
    group_service: &Arc<GroupService>,
    _tx: &mpsc::UnboundedSender<Result<ControlMessage, Status>>,
) {
    loop {
        let msg = match stream.message().await {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("southbound: stream error for node {node_id}: {e}");
                break;
            }
        };

        match msg.payload.clone() {
            Some(Payload::ConfigCommand(cc)) => {
                for sub in &cc.subscriptions_to_set {
                    if sub.connection_id == "n/a"
                        && let Err(e) = route_service
                            .add_route(Route {
                                source_node_id: ALL_NODES_ID.to_string(),
                                dest_node_id: node_id.to_string(),
                                component0: sub.component_0.clone(),
                                component1: sub.component_1.clone(),
                                component2: sub.component_2.clone(),
                                component_id: sub.id,
                                link_id: String::new(),
                            })
                            .await
                    {
                        tracing::error!("southbound: error adding route: {e}");
                    }
                }
                for sub in &cc.subscriptions_to_delete {
                    let route = Route {
                        source_node_id: ALL_NODES_ID.to_string(),
                        dest_node_id: node_id.to_string(),
                        component0: sub.component_0.clone(),
                        component1: sub.component_1.clone(),
                        component2: sub.component_2.clone(),
                        component_id: sub.id,
                        link_id: String::new(),
                    };
                    if let Err(e) = route_service.delete_route(route.clone()).await {
                        tracing::error!("southbound: error deleting route: {e}");
                        route_service
                            .requeue_route_for_source_node(node_id, route)
                            .await;
                    }
                }
            }
            Some(Payload::DeregisterNodeRequest(dr)) => {
                if let Some(node) = dr.node {
                    let nid = node.id;
                    cmd_handler
                        .update_connection_status(&nid, NodeStatus::NotConnected)
                        .await;
                    let _ = cmd_handler.remove_stream(&nid).await;
                    let resp = ControlMessage {
                        message_id: Uuid::new_v4().to_string(),
                        payload: Some(Payload::DeregisterNodeResponse(
                            crate::api::proto::controller::proto::v1::DeregisterNodeResponse {
                                success: true,
                                original_message_id: msg.message_id.clone(),
                            },
                        )),
                    };
                    if let Err(e) = cmd_handler.send_message(&nid, resp).await {
                        tracing::error!("southbound: error sending DeregisterNodeResponse: {e}");
                    }
                    return;
                }
            }
            Some(Payload::Ack(_))
            | Some(Payload::ConfigCommandAck(_))
            | Some(Payload::SubscriptionListResponse(_))
            | Some(Payload::ConnectionListResponse(_)) => {
                cmd_handler.response_received(node_id, msg).await;
            }
            Some(Payload::CreateChannelRequest(cr)) => {
                let channel_req =
                    crate::api::proto::controlplane::proto::v1::CreateChannelRequest {
                        moderators: cr.moderators.clone(),
                    };
                let orig_id = msg.message_id.clone();
                let success = match group_service.create_channel(channel_req).await {
                    Ok(_resp) => true,
                    Err(e) => {
                        tracing::error!("southbound: error creating channel: {e}");
                        false
                    }
                };
                let ack_msg = ControlMessage {
                    message_id: Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(Ack {
                        success,
                        original_message_id: orig_id,
                        ..Default::default()
                    })),
                };
                if let Err(e) = cmd_handler.send_message(node_id, ack_msg).await {
                    tracing::error!("southbound: error sending CreateChannelResponse: {e}");
                    break;
                }
            }
            Some(Payload::DeleteChannelRequest(dr)) => {
                let orig_id = msg.message_id.clone();
                let success = match group_service.delete_channel(dr).await {
                    Ok(ack) => ack.success,
                    Err(e) => {
                        tracing::error!("southbound: error deleting channel: {e}");
                        false
                    }
                };
                let ack_msg = ControlMessage {
                    message_id: Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(Ack {
                        success,
                        original_message_id: orig_id,
                        ..Default::default()
                    })),
                };
                if let Err(e) = cmd_handler.send_message(node_id, ack_msg).await {
                    tracing::error!("southbound: error sending ack: {e}");
                    break;
                }
            }
            Some(Payload::AddParticipantRequest(ar)) => {
                let orig_id = msg.message_id.clone();
                let success = match group_service.add_participant(ar).await {
                    Ok(ack) => ack.success,
                    Err(e) => {
                        tracing::error!("southbound: error adding participant: {e}");
                        false
                    }
                };
                let ack_msg = ControlMessage {
                    message_id: Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(Ack {
                        success,
                        original_message_id: orig_id,
                        ..Default::default()
                    })),
                };
                if let Err(e) = cmd_handler.send_message(node_id, ack_msg).await {
                    tracing::error!("southbound: error sending ack: {e}");
                    break;
                }
            }
            Some(Payload::DeleteParticipantRequest(dr)) => {
                let orig_id = msg.message_id.clone();
                let success = match group_service.delete_participant(dr).await {
                    Ok(ack) => ack.success,
                    Err(e) => {
                        tracing::error!("southbound: error deleting participant: {e}");
                        false
                    }
                };
                let ack_msg = ControlMessage {
                    message_id: Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(Ack {
                        success,
                        original_message_id: orig_id,
                        ..Default::default()
                    })),
                };
                if let Err(e) = cmd_handler.send_message(node_id, ack_msg).await {
                    tracing::error!("southbound: error sending ack: {e}");
                    break;
                }
            }
            Some(Payload::ListChannelRequest(lr)) => {
                let orig_id = msg.message_id.clone();
                match group_service.list_channels(lr).await {
                    Ok(mut resp) => {
                        resp.original_message_id = orig_id.clone();
                        let reply = ControlMessage {
                            message_id: Uuid::new_v4().to_string(),
                            payload: Some(Payload::ListChannelResponse(resp)),
                        };
                        if let Err(e) = cmd_handler.send_message(node_id, reply).await {
                            tracing::error!("southbound: error sending ListChannelResponse: {e}");
                            break;
                        }
                    }
                    Err(e) => tracing::error!("southbound: error listing channels: {e}"),
                }
            }
            Some(Payload::ListParticipantsRequest(lr)) => {
                let orig_id = msg.message_id.clone();
                match group_service.list_participants(lr).await {
                    Ok(mut resp) => {
                        resp.original_message_id = orig_id.clone();
                        let reply = ControlMessage {
                            message_id: Uuid::new_v4().to_string(),
                            payload: Some(Payload::ListParticipantsResponse(resp)),
                        };
                        if let Err(e) = cmd_handler.send_message(node_id, reply).await {
                            tracing::error!(
                                "southbound: error sending ListParticipantsResponse: {e}"
                            );
                            break;
                        }
                    }
                    Err(e) => tracing::error!("southbound: error listing participants: {e}"),
                }
            }
            _ => {
                tracing::debug!(
                    "southbound: unexpected payload from node {node_id}: {:?}",
                    msg.message_id
                );
            }
        }
    }

    // Stream ended or errored — mark disconnected.
    cmd_handler
        .update_connection_status(node_id, NodeStatus::NotConnected)
        .await;
    let _ = cmd_handler.remove_stream(node_id).await;
}

/// Parse proto `ConnectionDetails` into the DB model.
fn parse_conn_details(
    peer_host: &str,
    detail: &crate::api::proto::controller::proto::v1::ConnectionDetails,
) -> ConnectionDetails {
    let meta = detail.metadata.as_ref();
    let local_endpoint = meta
        .and_then(|m| m.fields.get("local_endpoint"))
        .and_then(|v| match &v.kind {
            Some(prost_types::value::Kind::StringValue(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });
    let external_endpoint = meta
        .and_then(|m| m.fields.get("external_endpoint"))
        .and_then(|v| match &v.kind {
            Some(prost_types::value::Kind::StringValue(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });
    let trust_domain = meta
        .and_then(|m| m.fields.get("trust_domain"))
        .and_then(|v| match &v.kind {
            Some(prost_types::value::Kind::StringValue(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });

    // Derive the effective endpoint: prefer local_endpoint, else peer host +
    // port from the proto endpoint field.
    let mut endpoint = local_endpoint.unwrap_or_else(|| peer_host.to_string());
    if let Some(port) = detail.endpoint.rsplit(':').next()
        && port.parse::<u16>().is_ok()
    {
        endpoint = format!("{endpoint}:{port}");
    }

    // Extract client_config from metadata if present.
    let client_config = meta
        .and_then(|m| m.fields.get("client_config"))
        .and_then(|v| match &v.kind {
            Some(prost_types::value::Kind::StructValue(s)) => {
                serde_json::to_value(struct_to_map(s)).ok()
            }
            Some(prost_types::value::Kind::StringValue(s)) => serde_json::from_str(s).ok(),
            _ => None,
        })
        .unwrap_or(serde_json::Value::Object(Default::default()));

    ConnectionDetails {
        endpoint,
        external_endpoint,
        trust_domain,
        mtls_required: detail.mtls_required,
        client_config,
    }
}

/// Convert a protobuf `Struct` to a `serde_json::Map`.
fn struct_to_map(s: &prost_types::Struct) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in &s.fields {
        map.insert(k.clone(), proto_value_to_json(v));
    }
    map
}

fn proto_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    match &v.kind {
        Some(prost_types::value::Kind::NullValue(_)) => serde_json::Value::Null,
        Some(prost_types::value::Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(prost_types::value::Kind::NumberValue(n)) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*n).unwrap_or_else(|| 0.into()))
        }
        Some(prost_types::value::Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(prost_types::value::Kind::StructValue(sv)) => {
            serde_json::Value::Object(struct_to_map(sv))
        }
        Some(prost_types::value::Kind::ListValue(lv)) => {
            serde_json::Value::Array(lv.values.iter().map(proto_value_to_json).collect())
        }
        None => serde_json::Value::Null,
    }
}
