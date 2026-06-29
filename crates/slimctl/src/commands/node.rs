// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use tokio::time::timeout as stream_timeout;
use tokio_stream::StreamExt;

use crate::client::get_controller_client;
use crate::proto::controller::proto::v1::{
    ConnectionDirection, ConnectionType, ControlMessage, control_message::Payload,
};
use slim_config::grpc::client::ClientConfig;

#[derive(Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(Subcommand)]
pub enum NodeCommand {
    /// Manage routes directly on a SLIM node
    Route(NodeRouteArgs),
    /// Manage connections directly on a SLIM node
    #[command(visible_alias = "conn")]
    Connection(NodeConnectionArgs),
}

// ── Route ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct NodeRouteArgs {
    #[command(subcommand)]
    pub command: NodeRouteCommand,
}

#[derive(Subcommand)]
pub enum NodeRouteCommand {
    /// List routes on a node
    #[command(visible_alias = "ls")]
    List,
}

// ── Connection ────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct NodeConnectionArgs {
    #[command(subcommand)]
    pub command: NodeConnectionCommand,
}

#[derive(Subcommand)]
pub enum NodeConnectionCommand {
    /// List active connections on a node
    #[command(visible_alias = "ls")]
    List,
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(args: &NodeArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        NodeCommand::Route(a) => run_route(a, opts).await,
        NodeCommand::Connection(a) => run_connection(a, opts).await,
    }
}

async fn run_route(args: &NodeRouteArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        NodeRouteCommand::List => route_list(opts).await,
    }
}

async fn run_connection(args: &NodeConnectionArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        NodeConnectionCommand::List => connection_list(opts).await,
    }
}

// ── Route commands ─────────────────────────────────────────────────────────────

async fn route_list(opts: &ClientConfig) -> Result<()> {
    let mut client = get_controller_client(opts).await?;
    let msg = ControlMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        payload: Some(Payload::RouteListRequest(
            crate::proto::controller::proto::v1::RouteListRequest {},
        )),
    };
    let req = tonic::Request::new(tokio_stream::once(msg));
    let mut stream = client.open_control_channel(req).await?.into_inner();
    loop {
        match stream_timeout(opts.request_timeout.into(), stream.next()).await {
            Err(_) => bail!("timeout waiting for route list response"),
            Ok(None) => break,
            Ok(Some(Err(e))) => bail!("stream error: {}", e),
            Ok(Some(Ok(msg))) => {
                if let Some(Payload::RouteListResponse(list_resp)) = msg.payload {
                    // Flatten: one row per (route, connection) pair.
                    let headers = ["ROUTE", "TYPE", "ENDPOINT", "LINK_ID"];
                    let mut rows: Vec<[String; 4]> = Vec::new();

                    for e in &list_resp.entries {
                        let route_name = e
                            .name
                            .as_ref()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "-".to_string());
                        if e.connections.is_empty() {
                            rows.push([route_name, "-".into(), "-".into(), "-".into()]);
                        } else {
                            for c in &e.connections {
                                let conn_type = match c.connection_type() {
                                    ConnectionType::Local => "Local",
                                    ConnectionType::Remote => "Remote",
                                    ConnectionType::Peer => "Peer",
                                    ConnectionType::Edge => "Edge",
                                };
                                let endpoint = if c.connection_type() == ConnectionType::Edge {
                                    "APP".to_string()
                                } else {
                                    c.peer_node_id.as_deref().unwrap_or("-").to_string()
                                };
                                let link_id = c.link_id.as_deref().unwrap_or("-").to_string();
                                rows.push([
                                    route_name.clone(),
                                    conn_type.into(),
                                    endpoint,
                                    link_id,
                                ]);
                            }
                        }
                    }

                    println!("{} route(s)\n", list_resp.entries.len());
                    if !rows.is_empty() {
                        let mut widths = headers.map(|h| h.len());
                        for row in &rows {
                            for (w, cell) in widths.iter_mut().zip(row.iter()) {
                                *w = (*w).max(cell.len());
                            }
                        }
                        println!(
                            "  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}",
                            headers[0],
                            headers[1],
                            headers[2],
                            headers[3],
                            w0 = widths[0],
                            w1 = widths[1],
                            w2 = widths[2],
                            w3 = widths[3],
                        );
                        let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
                        println!("  {}", "-".repeat(total));
                        for row in &rows {
                            println!(
                                "  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}",
                                row[0],
                                row[1],
                                row[2],
                                row[3],
                                w0 = widths[0],
                                w1 = widths[1],
                                w2 = widths[2],
                                w3 = widths[3],
                            );
                        }
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

// ── Connection commands ────────────────────────────────────────────────────────

async fn connection_list(opts: &ClientConfig) -> Result<()> {
    let mut client = get_controller_client(opts).await?;
    let msg = ControlMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        payload: Some(Payload::ConnectionListRequest(
            crate::proto::controller::proto::v1::ConnectionListRequest {},
        )),
    };
    let req = tonic::Request::new(tokio_stream::once(msg));
    let mut stream = client.open_control_channel(req).await?.into_inner();
    loop {
        match stream_timeout(opts.request_timeout.into(), stream.next()).await {
            Err(_) => bail!("timeout waiting for connection list response"),
            Ok(None) => break,
            Ok(Some(Err(e))) => bail!("stream error: {}", e),
            Ok(Some(Ok(msg))) => {
                if let Some(Payload::ConnectionListResponse(list_resp)) = msg.payload {
                    let entries = &list_resp.entries;
                    println!("{} connection(s)\n", entries.len());
                    if !entries.is_empty() {
                        let headers = ["CONN_ID", "TYPE", "DIRECTION", "ENDPOINT", "LINK_ID"];
                        let rows: Vec<_> = entries
                            .iter()
                            .map(|e| {
                                let conn_type = match e.connection_type() {
                                    ConnectionType::Local => "Local",
                                    ConnectionType::Remote => "Remote",
                                    ConnectionType::Peer => "Peer",
                                    ConnectionType::Edge => "Edge",
                                };
                                let direction = match e.direction() {
                                    ConnectionDirection::Outgoing => "Outgoing",
                                    ConnectionDirection::Incoming => "Incoming",
                                };
                                let endpoint = if e.connection_type() == ConnectionType::Edge {
                                    "APP".to_string()
                                } else {
                                    e.peer_node_id.as_deref().unwrap_or("-").to_string()
                                };
                                let link_id = e.link_id.as_deref().unwrap_or("-").to_string();
                                (
                                    e.id.to_string(),
                                    conn_type.to_string(),
                                    direction.to_string(),
                                    endpoint,
                                    link_id,
                                )
                            })
                            .collect();

                        let mut widths = headers.map(|h| h.len());
                        for (id, ct, dir, ep, lid) in &rows {
                            widths[0] = widths[0].max(id.len());
                            widths[1] = widths[1].max(ct.len());
                            widths[2] = widths[2].max(dir.len());
                            widths[3] = widths[3].max(ep.len());
                            widths[4] = widths[4].max(lid.len());
                        }
                        println!(
                            "  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
                            headers[0],
                            headers[1],
                            headers[2],
                            headers[3],
                            headers[4],
                            w0 = widths[0],
                            w1 = widths[1],
                            w2 = widths[2],
                            w3 = widths[3],
                            w4 = widths[4],
                        );
                        let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
                        println!("  {}", "-".repeat(total));
                        for (id, ct, dir, ep, lid) in &rows {
                            println!(
                                "  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
                                id,
                                ct,
                                dir,
                                ep,
                                lid,
                                w0 = widths[0],
                                w1 = widths[1],
                                w2 = widths[2],
                                w3 = widths[3],
                                w4 = widths[4],
                            );
                        }
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::TcpListenerStream;

    use crate::proto::controller::proto::v1::{
        ConnectionListResponse, ControlMessage, RouteListResponse,
        control_message::Payload,
        controller_service_server::{ControllerService, ControllerServiceServer},
    };
    use slim_config::grpc::client::ClientConfig;

    use super::*;

    struct MockControllerSvc;

    #[tonic::async_trait]
    impl ControllerService for MockControllerSvc {
        type OpenControlChannelStream = std::pin::Pin<
            Box<
                dyn tonic::codegen::tokio_stream::Stream<
                        Item = Result<ControlMessage, tonic::Status>,
                    > + Send
                    + 'static,
            >,
        >;

        async fn open_control_channel(
            &self,
            request: tonic::Request<tonic::Streaming<ControlMessage>>,
        ) -> Result<tonic::Response<Self::OpenControlChannelStream>, tonic::Status> {
            let mut stream = request.into_inner();
            let msg = stream
                .next()
                .await
                .ok_or_else(|| tonic::Status::internal("no message received"))?
                .map_err(|e| tonic::Status::internal(e.to_string()))?;

            let resp_payload = match msg.payload {
                Some(Payload::RouteListRequest(_)) => {
                    Payload::RouteListResponse(RouteListResponse {
                        original_message_id: msg.message_id.clone(),
                        entries: vec![],
                        done: true,
                    })
                }
                Some(Payload::ConnectionListRequest(_)) => {
                    Payload::ConnectionListResponse(ConnectionListResponse {
                        original_message_id: msg.message_id.clone(),
                        entries: vec![],
                        done: true,
                    })
                }
                _ => return Err(tonic::Status::invalid_argument("unknown message type")),
            };

            let resp_msg = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(resp_payload),
            };

            Ok(tonic::Response::new(Box::pin(tokio_stream::once(Ok(
                resp_msg,
            )))))
        }
    }

    async fn spawn_mock_node_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpListenerStream::new(listener);
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(ControllerServiceServer::new(MockControllerSvc))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        format!("{}:{}", addr.ip(), addr.port())
    }

    fn make_opts(addr: &str) -> ClientConfig {
        slim_config::grpc::client::ClientConfig::with_endpoint(&format!("http://{}", addr))
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure())
            .with_request_timeout(Duration::from_secs(5))
    }

    #[tokio::test]
    async fn route_list_succeeds() {
        let addr = spawn_mock_node_server().await;
        route_list(&make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn connection_list_succeeds() {
        let addr = spawn_mock_node_server().await;
        connection_list(&make_opts(&addr)).await.unwrap();
    }

    // ── error-handling mock services ─────────────────────────────────────────

    /// Returns a gRPC status error immediately (before sending any stream items).
    struct ErrorControllerSvc;

    #[tonic::async_trait]
    impl ControllerService for ErrorControllerSvc {
        type OpenControlChannelStream = std::pin::Pin<
            Box<
                dyn tonic::codegen::tokio_stream::Stream<
                        Item = Result<ControlMessage, tonic::Status>,
                    > + Send
                    + 'static,
            >,
        >;

        async fn open_control_channel(
            &self,
            _request: tonic::Request<tonic::Streaming<ControlMessage>>,
        ) -> Result<tonic::Response<Self::OpenControlChannelStream>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }
    }

    /// Returns an `Ok` stream whose first item is itself an error.
    struct StreamErrorControllerSvc;

    #[tonic::async_trait]
    impl ControllerService for StreamErrorControllerSvc {
        type OpenControlChannelStream = std::pin::Pin<
            Box<
                dyn tonic::codegen::tokio_stream::Stream<
                        Item = Result<ControlMessage, tonic::Status>,
                    > + Send
                    + 'static,
            >,
        >;

        async fn open_control_channel(
            &self,
            _request: tonic::Request<tonic::Streaming<ControlMessage>>,
        ) -> Result<tonic::Response<Self::OpenControlChannelStream>, tonic::Status> {
            let items: Vec<Result<ControlMessage, tonic::Status>> =
                vec![Err(tonic::Status::internal("stream item error"))];
            Ok(tonic::Response::new(Box::pin(tokio_stream::iter(items))))
        }
    }

    async fn spawn_svc<S>(svc: S) -> String
    where
        S: crate::proto::controller::proto::v1::controller_service_server::ControllerService,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpListenerStream::new(listener);
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(ControllerServiceServer::new(svc))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        format!("{}:{}", addr.ip(), addr.port())
    }

    // ── gRPC-level error tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn route_list_grpc_error_propagates() {
        let addr = spawn_svc(ErrorControllerSvc).await;
        assert!(route_list(&make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn connection_list_grpc_error_propagates() {
        let addr = spawn_svc(ErrorControllerSvc).await;
        assert!(connection_list(&make_opts(&addr)).await.is_err());
    }

    // ── stream-item error tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn route_list_stream_error_propagates() {
        let addr = spawn_svc(StreamErrorControllerSvc).await;
        let err = route_list(&make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("stream error"));
    }

    #[tokio::test]
    async fn connection_list_stream_error_propagates() {
        let addr = spawn_svc(StreamErrorControllerSvc).await;
        let err = connection_list(&make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("stream error"));
    }
}
