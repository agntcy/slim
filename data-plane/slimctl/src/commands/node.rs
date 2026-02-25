// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use tokio::time::timeout as stream_timeout;
use tokio_stream::StreamExt;

use crate::client::get_controller_client;
use crate::config::ResolvedOpts;
use crate::proto::controller::proto::v1::{
    ConfigurationCommand, ControlMessage, Subscription, control_message::Payload,
};
use crate::utils::{VIA_KEYWORD, parse_config_file, parse_endpoint, parse_route};

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
    #[command(alias = "conn")]
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
    #[command(alias = "ls")]
    List,
    /// Add a route to a node
    ///
    /// Usage: node route add <org/ns/agent/id> via <config.json>
    Add {
        /// Route in org/namespace/agentname/agentid format
        route: String,
        /// Literal keyword "via"
        via: String,
        /// Path to JSON connection config file
        config_file: String,
    },
    /// Delete a route from a node
    ///
    /// Usage: node route del <org/ns/agent/id> via <http|https://host:port>
    Del {
        /// Route in org/namespace/agentname/agentid format
        route: String,
        /// Literal keyword "via"
        via: String,
        /// Endpoint URL (http|https://host:port)
        endpoint: String,
    },
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
    #[command(alias = "ls")]
    List,
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(args: &NodeArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        NodeCommand::Route(a) => run_route(a, opts).await,
        NodeCommand::Connection(a) => run_connection(a, opts).await,
    }
}

async fn run_route(args: &NodeRouteArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        NodeRouteCommand::List => route_list(opts).await,
        NodeRouteCommand::Add {
            route,
            via,
            config_file,
        } => route_add(route, via, config_file, opts).await,
        NodeRouteCommand::Del {
            route,
            via,
            endpoint,
        } => route_del(route, via, endpoint, opts).await,
    }
}

async fn run_connection(args: &NodeConnectionArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        NodeConnectionCommand::List => connection_list(opts).await,
    }
}

// ── Route commands ─────────────────────────────────────────────────────────────

async fn route_list(opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_controller_client(opts).await?;
    let msg = ControlMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        payload: Some(Payload::SubscriptionListRequest(
            crate::proto::controller::proto::v1::SubscriptionListRequest {},
        )),
    };
    let req = tonic::Request::new(tokio_stream::once(msg));
    let mut stream = client.open_control_channel(req).await?.into_inner();
    loop {
        match stream_timeout(opts.timeout, stream.next()).await {
            Err(_) => bail!("timeout waiting for route list response"),
            Ok(None) => break,
            Ok(Some(Err(e))) => bail!("stream error: {}", e),
            Ok(Some(Ok(msg))) => {
                if let Some(Payload::SubscriptionListResponse(list_resp)) = msg.payload {
                    for e in &list_resp.entries {
                        let local_names: Vec<String> = e
                            .local_connections
                            .iter()
                            .map(|c| format!("local:{}:{}", c.id, c.config_data))
                            .collect();
                        let remote_names: Vec<String> = e
                            .remote_connections
                            .iter()
                            .map(|c| format!("remote:{}:{}", c.id, c.config_data))
                            .collect();
                        println!(
                            "{}/{}/{} id={} local={:?} remote={:?}",
                            e.component_0,
                            e.component_1,
                            e.component_2,
                            e.id.map_or_else(|| "None".to_string(), |id| id.to_string()),
                            local_names,
                            remote_names
                        );
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn route_add(route: &str, via: &str, config_file: &str, opts: &ResolvedOpts) -> Result<()> {
    if !via.eq_ignore_ascii_case(VIA_KEYWORD) {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    let (org, ns, agent_type, agent_id) = parse_route(route)?;
    let conn = parse_config_file(config_file)?;
    let subscription = Subscription {
        component_0: org,
        component_1: ns,
        component_2: agent_type,
        id: Some(agent_id),
        connection_id: conn.connection_id.clone(),
        node_id: None,
    };
    let msg = ControlMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: vec![conn],
            subscriptions_to_set: vec![subscription],
            subscriptions_to_delete: vec![],
        })),
    };
    let mut client = get_controller_client(opts).await?;
    let req = tonic::Request::new(tokio_stream::once(msg));
    let mut stream = client.open_control_channel(req).await?.into_inner();
    match stream_timeout(opts.timeout, stream.next()).await {
        Err(_) => bail!("timeout waiting for ACK"),
        Ok(None) => bail!("stream closed before receiving ACK"),
        Ok(Some(Err(e))) => bail!("error receiving ACK: {}", e),
        Ok(Some(Ok(ack_msg))) => {
            if let Some(Payload::ConfigCommandAck(a)) = ack_msg.payload {
                for cs in &a.connections_status {
                    if cs.success {
                        println!("connection successfully applied: {}", cs.connection_id);
                    } else {
                        println!(
                            "failed to create connection {}: {}",
                            cs.connection_id, cs.error_msg
                        );
                    }
                }
                for ss in &a.subscriptions_status {
                    if ss.success {
                        println!("route added successfully: {:?}", ss.subscription);
                    } else {
                        println!(
                            "failed to add route {:?}: {}",
                            ss.subscription, ss.error_msg
                        );
                    }
                }
            } else {
                bail!("unexpected response type (not an ACK): {:?}", ack_msg);
            }
        }
    }
    Ok(())
}

async fn route_del(route: &str, via: &str, endpoint: &str, opts: &ResolvedOpts) -> Result<()> {
    if !via.eq_ignore_ascii_case(VIA_KEYWORD) {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    let (org, ns, agent_type, agent_id) = parse_route(route)?;
    let (_, conn_id) = parse_endpoint(endpoint)?;
    let subscription = Subscription {
        component_0: org,
        component_1: ns,
        component_2: agent_type,
        id: Some(agent_id),
        connection_id: conn_id,
        node_id: None,
    };
    let msg = ControlMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: vec![],
            subscriptions_to_set: vec![],
            subscriptions_to_delete: vec![subscription],
        })),
    };
    let mut client = get_controller_client(opts).await?;
    let req = tonic::Request::new(tokio_stream::once(msg));
    let mut stream = client.open_control_channel(req).await?.into_inner();
    match stream_timeout(opts.timeout, stream.next()).await {
        Err(_) => bail!("timeout waiting for ACK"),
        Ok(None) => bail!("stream closed before receiving ACK"),
        Ok(Some(Err(e))) => bail!("error receiving ACK: {}", e),
        Ok(Some(Ok(ack_msg))) => {
            if let Some(Payload::ConfigCommandAck(a)) = ack_msg.payload {
                for ss in &a.subscriptions_status {
                    if ss.success {
                        println!("route deleted successfully: {:?}", ss.subscription);
                    } else {
                        println!(
                            "failed to delete route {:?}: {}",
                            ss.subscription, ss.error_msg
                        );
                    }
                }
            } else {
                bail!("unexpected response type (not an ACK): {:?}", ack_msg);
            }
        }
    }
    Ok(())
}

// ── Connection commands ────────────────────────────────────────────────────────

async fn connection_list(opts: &ResolvedOpts) -> Result<()> {
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
        match stream_timeout(opts.timeout, stream.next()).await {
            Err(_) => bail!("timeout waiting for connection list response"),
            Ok(None) => break,
            Ok(Some(Err(e))) => bail!("stream error: {}", e),
            Ok(Some(Ok(msg))) => {
                if let Some(Payload::ConnectionListResponse(list_resp)) = msg.payload {
                    for e in &list_resp.entries {
                        println!("id={} {}", e.id, e.config_data);
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
    use std::io::Write;
    use std::time::Duration;

    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::TcpListenerStream;

    use crate::config::ResolvedOpts;
    use crate::proto::controller::proto::v1::{
        ConfigurationCommandAck, ConnectionListResponse, ControlMessage, SubscriptionListResponse,
        control_message::Payload,
        controller_service_server::{ControllerService, ControllerServiceServer},
    };

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
                Some(Payload::SubscriptionListRequest(_)) => {
                    Payload::SubscriptionListResponse(SubscriptionListResponse {
                        original_message_id: msg.message_id.clone(),
                        entries: vec![],
                    })
                }
                Some(Payload::ConnectionListRequest(_)) => {
                    Payload::ConnectionListResponse(ConnectionListResponse {
                        original_message_id: msg.message_id.clone(),
                        entries: vec![],
                    })
                }
                Some(Payload::ConfigCommand(_)) => {
                    Payload::ConfigCommandAck(ConfigurationCommandAck {
                        original_message_id: msg.message_id.clone(),
                        connections_status: vec![],
                        subscriptions_status: vec![],
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

    fn make_opts(addr: &str) -> ResolvedOpts {
        ResolvedOpts {
            server: addr.to_string(),
            timeout: Duration::from_secs(5),
            tls_insecure: true,
            tls_insecure_skip_verify: false,
            tls_ca_file: String::new(),
            tls_cert_file: String::new(),
            tls_key_file: String::new(),
            basic_auth_creds: String::new(),
        }
    }

    #[test]
    fn route_add_invalid_via_fails() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(route_add(
            "a/b/c/0",
            "not_via",
            "config.json",
            &make_opts("127.0.0.1:1"),
        ));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("via"));
    }

    #[test]
    fn route_del_invalid_via_fails() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(route_del(
            "a/b/c/0",
            "wrong",
            "http://host:80",
            &make_opts("127.0.0.1:1"),
        ));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("via"));
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

    #[tokio::test]
    async fn route_add_via_mock_server() {
        let addr = spawn_mock_node_server().await;
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": "http://127.0.0.1:8080"}}"#).unwrap();
        let path = f.path().to_str().unwrap().to_string();
        route_add("a/b/c/0", "via", &path, &make_opts(&addr))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn route_del_via_mock_server() {
        let addr = spawn_mock_node_server().await;
        route_del("a/b/c/0", "via", "http://127.0.0.1:8080", &make_opts(&addr))
            .await
            .unwrap();
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

    /// Returns a ConfigCommandAck whose per-entry status flags are all false
    /// (negative ACK), to exercise the "failed to …" print branches.
    struct NackControllerSvc;

    #[tonic::async_trait]
    impl ControllerService for NackControllerSvc {
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
            use crate::proto::controller::proto::v1::{
                ConfigurationCommandAck, ConnectionAck, SubscriptionAck,
            };
            let mut stream = request.into_inner();
            let msg = stream
                .next()
                .await
                .ok_or_else(|| tonic::Status::internal("no message"))?
                .map_err(|e| tonic::Status::internal(e.to_string()))?;

            let ack = ConfigurationCommandAck {
                original_message_id: msg.message_id.clone(),
                connections_status: vec![ConnectionAck {
                    connection_id: "c1".to_string(),
                    success: false,
                    error_msg: "connection failed".to_string(),
                }],
                subscriptions_status: vec![SubscriptionAck {
                    subscription: None,
                    success: false,
                    error_msg: "subscription failed".to_string(),
                }],
            };
            let resp = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(Payload::ConfigCommandAck(ack)),
            };
            Ok(tonic::Response::new(Box::pin(tokio_stream::once(Ok(resp)))))
        }
    }

    /// Returns an unexpected payload type (not a ConfigCommandAck) in response
    /// to a ConfigCommand, exercising the `bail!("unexpected response type …")` arm.
    struct UnexpectedPayloadControllerSvc;

    #[tonic::async_trait]
    impl ControllerService for UnexpectedPayloadControllerSvc {
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
            use crate::proto::controller::proto::v1::SubscriptionListResponse;
            let resp = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(Payload::SubscriptionListResponse(
                    SubscriptionListResponse::default(),
                )),
            };
            Ok(tonic::Response::new(Box::pin(tokio_stream::once(Ok(resp)))))
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

    #[tokio::test]
    async fn route_add_grpc_error_propagates() {
        let addr = spawn_svc(ErrorControllerSvc).await;
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": "http://127.0.0.1:8080"}}"#).unwrap();
        let path = f.path().to_str().unwrap().to_string();
        assert!(
            route_add("a/b/c/0", "via", &path, &make_opts(&addr))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn route_del_grpc_error_propagates() {
        let addr = spawn_svc(ErrorControllerSvc).await;
        assert!(
            route_del("a/b/c/0", "via", "http://127.0.0.1:8080", &make_opts(&addr))
                .await
                .is_err()
        );
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

    // ── negative-ACK tests (success = false per entry) ───────────────────────

    #[tokio::test]
    async fn route_add_negative_ack_prints_failure() {
        let addr = spawn_svc(NackControllerSvc).await;
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": "http://127.0.0.1:8080"}}"#).unwrap();
        let path = f.path().to_str().unwrap().to_string();
        // The client prints the failure but still returns Ok
        assert!(
            route_add("a/b/c/0", "via", &path, &make_opts(&addr))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn route_del_negative_ack_prints_failure() {
        let addr = spawn_svc(NackControllerSvc).await;
        // The client prints the failure but still returns Ok
        assert!(
            route_del("a/b/c/0", "via", "http://127.0.0.1:8080", &make_opts(&addr))
                .await
                .is_ok()
        );
    }

    // ── unexpected-payload tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn route_add_unexpected_payload_fails() {
        let addr = spawn_svc(UnexpectedPayloadControllerSvc).await;
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": "http://127.0.0.1:8080"}}"#).unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let err = route_add("a/b/c/0", "via", &path, &make_opts(&addr))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unexpected response type"));
    }

    #[tokio::test]
    async fn route_del_unexpected_payload_fails() {
        let addr = spawn_svc(UnexpectedPayloadControllerSvc).await;
        let err = route_del("a/b/c/0", "via", "http://127.0.0.1:8080", &make_opts(&addr))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unexpected response type"));
    }
}
