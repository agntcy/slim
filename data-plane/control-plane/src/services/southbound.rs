// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    Ack, ControlMessage, control_message::Payload, controller_service_server::ControllerService,
};
use crate::db::{ConnectionDetails, Node, SharedDb};
use crate::error::{Error, Result};
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus};
use crate::route_service::{ALL_NODES_ID, Route, RouteService};

const REGISTER_TIMEOUT_SECS: u64 = 15;

pub struct SouthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
}

impl SouthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
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

            handle_node_messages(stream, &registered_node_id, &cmd_handler, &route_service).await;
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
    cmd_handler: &DefaultNodeCommandHandler,
    route_service: &RouteService,
    tx: &mpsc::UnboundedSender<Result<ControlMessage, Status>>,
) -> Result<String> {
    let msg = tokio::time::timeout(Duration::from_secs(REGISTER_TIMEOUT_SECS), stream.message())
        .await
        .map_err(|e| {
            Error::InvalidInput(format!("timeout waiting for register node request: {e}"))
        })?
        .map_err(|e| Error::InvalidInput(format!("stream error: {e}")))?;

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
        .await?;

    cmd_handler.add_stream(&node_id, tx.clone()).await;

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
        .node_registered(&node_id, conn_details_updated, reg_req.connections)
        .await;

    Ok(node_id)
}

/// Main message loop after the node has been registered.
async fn handle_node_messages(
    mut stream: Streaming<ControlMessage>,
    node_id: &str,
    cmd_handler: &DefaultNodeCommandHandler,
    route_service: &RouteService,
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

        match msg.payload {
            Some(Payload::ConfigCommand(cc)) => {
                for sub in &cc.subscriptions_to_set {
                    if sub.link_id.is_none()
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
                        tracing::debug!("southbound: error adding route: {e}");
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
                    if let Err(e) = route_service.delete_route(route).await {
                        tracing::debug!("southbound: error deleting route: {e}");
                    }
                }
            }
            Some(Payload::DeregisterNodeRequest(dr)) => {
                if let Some(node) = dr.node {
                    let nid = node.id;
                    // Send the response BEFORE removing the stream — once the
                    // stream is gone send_message fails immediately.
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
                    cmd_handler
                        .update_connection_status(&nid, NodeStatus::NotConnected)
                        .await;
                    let _ = cmd_handler.remove_stream(&nid).await;
                    // Clean up DB state for the deregistering node.
                    route_service.node_deregistered(&nid).await;
                    return;
                }
            }
            Some(Payload::Ack(_))
            | Some(Payload::ConfigCommandAck(_))
            | Some(Payload::SubscriptionListResponse(_))
            | Some(Payload::ConnectionListResponse(_)) => {
                cmd_handler.response_received(node_id, msg).await;
            }
            _ => {
                tracing::debug!(
                    "southbound: unexpected payload from node {node_id}: {:?}",
                    msg.message_id
                );
            }
        }
    }

    // Stream ended or errored — mark disconnected and clean up.
    let _ = cmd_handler.remove_stream(node_id).await;
    // Clean up the per-node lock entry so node_locks does not grow without
    // bound for crash-disconnected nodes that never re-register.
    route_service.remove_node_lock(node_id).await;
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
    let spire_socket_path = meta
        .and_then(|m| m.fields.get("spire_socket_path"))
        .and_then(|v| match &v.kind {
            Some(prost_types::value::Kind::StringValue(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });

    let spire_mtls = if detail.mtls_required {
        Some(crate::db::SpireMtls {
            socket_path: spire_socket_path.unwrap_or_default(),
            trust_domain,
        })
    } else {
        spire_socket_path.map(|p| crate::db::SpireMtls {
            socket_path: p,
            trust_domain,
        })
    };

    // Derive the effective endpoint: prefer local_endpoint, else peer host +
    // port from the proto endpoint field.
    let mut endpoint = local_endpoint.unwrap_or_else(|| peer_host.to_string());
    if let Some(port) = detail.endpoint.rsplit(':').next()
        && port.parse::<u16>().is_ok()
    {
        endpoint = format!("{endpoint}:{port}");
    }

    ConnectionDetails {
        endpoint,
        external_endpoint,
        spire_mtls,
    }
}
