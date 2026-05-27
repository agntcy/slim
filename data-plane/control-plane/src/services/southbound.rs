// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::Mutex;
use slim_datapath::api::{NameId, ProtoName};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    Connection, ControlMessage, RegisterNodeResponse, Route as ProtoRoute,
    control_message::Payload, controller_service_server::ControllerService,
};
use crate::db::{ConnectionDetails, LinkStatus, Node, RouteStatus, SharedDb};
use crate::error::{Error, Result};
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus};
use crate::route_service::{ALL_NODES_ID, RouteService};

const REGISTER_TIMEOUT_SECS: u64 = 15;

pub type SharedDrain = Arc<Mutex<Option<drain::Watch>>>;

pub struct SouthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
    drain: SharedDrain,
}

impl SouthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
        drain: SharedDrain,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
            drain,
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

        let db = self.db.clone();
        let cmd_handler = self.cmd_handler.clone();
        let route_service = self.route_service.clone();
        let drain = self.drain.lock().clone();

        tokio::spawn(async move {
            let (registered_node_id, stream_epoch) = match receive_register(
                &mut stream,
                &peer_addr,
                &db,
                &cmd_handler,
                &route_service,
                &tx,
            )
            .await
            {
                Ok(r) => r,
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
                stream_epoch,
                &cmd_handler,
                &route_service,
                drain,
            )
            .await;
        });

        Ok(Response::new(UnboundedReceiverStream::new(rx)))
    }
}

/// Wait for the RegisterNodeRequest with a timeout, save the node, register
/// the stream, and return the (node ID, stream epoch). Returns empty string
/// and epoch 0 if no register message arrived.
async fn receive_register(
    stream: &mut Streaming<ControlMessage>,
    peer_host: &str,
    db: &SharedDb,
    cmd_handler: &DefaultNodeCommandHandler,
    route_service: &RouteService,
    tx: &mpsc::UnboundedSender<Result<ControlMessage, Status>>,
) -> Result<(String, u64)> {
    let msg = tokio::time::timeout(Duration::from_secs(REGISTER_TIMEOUT_SECS), stream.message())
        .await
        .map_err(|e| {
            Error::InvalidInput(format!("timeout waiting for register node request: {e}"))
        })?
        .map_err(|e| Error::InvalidInput(format!("stream error: {e}")))?;

    let msg = match msg {
        Some(m) => m,
        None => return Ok((String::new(), 0)),
    };

    let request_message_id = msg.message_id.clone();

    let reg_req = match msg.payload {
        Some(Payload::RegisterNodeRequest(r)) => r,
        other => {
            return Err(Error::InvalidInput(format!(
                "expected RegisterNodeRequest, got {:?}",
                other.as_ref().map(std::mem::discriminant)
            )));
        }
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
            created_at: std::time::SystemTime::now(),
            last_updated: std::time::SystemTime::now(),
        })
        .await?;

    let stream_epoch = cmd_handler.add_stream(&node_id, tx.clone()).await;

    // Build the current desired state (links + routes) for this node from
    // what is already in the DB. Send the response BEFORE calling
    // node_registered so the DP exits its registration wait loop before
    // the reconciler can send follow-up commands.
    let connections = build_node_connections(db, route_service, &node_id).await;
    let routes = build_node_routes(db, &node_id).await;

    let response = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::RegisterNodeResponse(RegisterNodeResponse {
            original_message_id: request_message_id,
            success: true,
            connections,
            routes,
        })),
    };
    tx.send(Ok(response)).ok();

    // Now trigger link/route reconciliation. The DP has already received the
    // response and entered its event loop, so reconciler messages will be
    // handled correctly.
    route_service
        .node_registered(
            &node_id,
            conn_details_updated,
            reg_req.connections,
            reg_req.routes,
        )
        .await;

    Ok((node_id, stream_epoch))
}

/// Build desired connections for a node (outgoing links from this node).
/// Uses the destination node's current endpoint rather than the potentially
/// stale value stored in the link record.
async fn build_node_connections(
    db: &SharedDb,
    route_service: &RouteService,
    node_id: &str,
) -> Vec<Connection> {
    let mut connections = Vec::new();
    let links = match db.get_links_for_node(node_id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("build_node_connections: {e}");
            return connections;
        }
    };
    for link in links {
        if link.source_node_id != node_id || link.status == LinkStatus::Deleted {
            continue;
        }
        if link.link_id.is_empty() {
            continue;
        }
        let mut config = match route_service
            .get_connection_details(&link.source_node_id, &link.dest_node_id)
            .await
        {
            Ok((_endpoint, cfg)) => cfg,
            Err(_) => continue,
        };
        config.link_id = link.link_id.clone();
        let config_data = match serde_json::to_string(&config) {
            Ok(d) => d,
            Err(_) => continue,
        };
        connections.push(Connection {
            link_id: link.link_id.clone(),
            config_data,
        });
    }
    connections
}

/// Build desired routes for a node (routes sourced from this node).
async fn build_node_routes(db: &SharedDb, node_id: &str) -> Vec<ProtoRoute> {
    let mut routes = Vec::new();
    let db_routes = match db.get_routes_for_node(node_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("build_node_routes: {e}");
            return routes;
        }
    };
    for route in db_routes {
        if route.status == RouteStatus::Deleted || route.link_id.is_none() {
            continue;
        }
        let name = ProtoName::from_strings([
                    &route.component0,
                    &route.component1,
                    &route.component2,
                ])
                .with_id(
                    route.component_id
                        .as_deref()
                        .and_then(|s| uuid::Uuid::parse_str(s).ok())
                        .map(|u| u.as_u128())
                        .unwrap_or(NameId::NULL_COMPONENT),
                );
        routes.push(ProtoRoute {
            name: Some(name),
            link_id: route.link_id.clone(),
            ..Default::default()
        });
    }
    routes
}

/// Main message loop after the node has been registered.
async fn handle_node_messages(
    mut stream: Streaming<ControlMessage>,
    node_id: &str,
    stream_epoch: u64,
    cmd_handler: &DefaultNodeCommandHandler,
    route_service: &RouteService,
    drain: Option<drain::Watch>,
) {
    let shutdown = drain.map(|d| d.signaled());
    tokio::pin!(shutdown);

    loop {
        let msg = tokio::select! {
            result = stream.message() => {
                match result {
                    Ok(Some(m)) => m,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::error!("southbound: stream error for node {node_id}: {e}");
                        break;
                    }
                }
            }
            _ = async { match shutdown.as_mut().as_pin_mut() {
                Some(fut) => fut.await,
                None => std::future::pending().await,
            }} => {
                tracing::info!("southbound: drain signaled, closing stream for node {node_id}");
                break;
            }
        };

        match msg.payload {
            Some(Payload::ConfigCommand(cc)) => {
                for sub in &cc.routes_to_set {
                    if sub.link_id.is_none()
                        && let Err(e) = route_service.add_route(ALL_NODES_ID, node_id, sub).await
                    {
                        tracing::debug!("southbound: error adding route: {e}");
                    }
                }
                for sub in &cc.routes_to_delete {
                    if let Err(e) = route_service.delete_route(ALL_NODES_ID, node_id, sub).await {
                        tracing::debug!("southbound: error deleting route: {e}");
                    }
                }
            }
            Some(Payload::DeregisterNodeRequest(_)) => {
                // Always use the authenticated node_id from registration,
                // never the ID from the message payload.
                let resp = ControlMessage {
                    message_id: Uuid::new_v4().to_string(),
                    payload: Some(Payload::DeregisterNodeResponse(
                        crate::api::proto::controller::proto::v1::DeregisterNodeResponse {
                            success: true,
                            original_message_id: msg.message_id.clone(),
                        },
                    )),
                };
                if let Err(e) = cmd_handler.send_message(node_id, resp).await {
                    tracing::error!("southbound: error sending DeregisterNodeResponse: {e}");
                }
                cmd_handler
                    .update_connection_status(node_id, NodeStatus::NotConnected)
                    .await;
                let _ = cmd_handler.remove_stream(node_id, stream_epoch).await;
                route_service.node_deregistered(node_id).await;
                return;
            }
            Some(Payload::Ack(_))
            | Some(Payload::ConfigCommandAck(_))
            | Some(Payload::RouteListResponse(_))
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
    // Only remove if our epoch is still current (a newer connection may have
    // already replaced our stream).
    let _ = cmd_handler.remove_stream(node_id, stream_epoch).await;
    // Do NOT call remove_node_lock here: if the node already reconnected, the
    // new session may have re-created the lock entry, and removing it would
    // defeat the disconnect-reconnect serialization.  The entry is cleaned up
    // by node_deregistered on graceful deregister, or reused on reconnect.
}

/// Parse proto `ConnectionDetails` into the DB model.
fn parse_conn_details(
    peer_host: &str,
    detail: &crate::api::proto::controller::proto::v1::ConnectionDetails,
) -> ConnectionDetails {
    let spire_mtls = detail.spire_mtls.as_ref().map(|s| crate::db::SpireMtls {
        socket_path: s.socket_path.clone(),
        trust_domain: s.trust_domain.clone(),
    });

    // Derive the effective endpoint: if the proto endpoint contains a wildcard
    // address (0.0.0.0 or [::]), substitute the peer host.
    let mut endpoint = detail.endpoint.clone();
    if let Some((host, port)) = endpoint.rsplit_once(':') {
        let host_trimmed = host.trim_start_matches('[').trim_end_matches(']');
        if host_trimmed == "0.0.0.0" || host_trimmed == "::" || host_trimmed.is_empty() {
            endpoint = format!("{peer_host}:{port}");
        }
    }

    ConnectionDetails {
        endpoint,
        external_endpoint: detail.external_endpoint.clone(),
        spire_mtls,
    }
}
