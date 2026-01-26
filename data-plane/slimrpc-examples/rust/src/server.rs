// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod common;

use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::Parser;
use common::{create_local_app, RpcAppConfig, RpcAppConnection};
use futures::stream;
use futures::StreamExt;
use slim_bindings::slimrpc::{
    RequestStream, ResponseStream, RpcHandler, StreamStreamHandler, StreamUnaryHandler,
    UnaryStreamHandler, UnaryUnaryHandler,
};
use slim_bindings::{
    RpcMessageContext as MessageContext, RpcServer, RpcSessionContext as SessionContext,
};
use std::collections::HashMap;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "slimrpc-server")]
#[command(about = "SLIMRpc server example", long_about = None)]
struct Args {
    /// Local identity (org/namespace/app)
    #[arg(short, long, default_value = "agntcy/ns/server")]
    local: String,

    /// SLIM endpoint
    #[arg(short, long, default_value = "http://localhost:46357")]
    endpoint: String,

    /// Shared secret for authentication
    #[arg(short, long, default_value = "abcde-12345-fedcb-67890-deadc000")]
    secret: String,

    /// Enable OpenTelemetry
    #[arg(long)]
    opentelemetry: bool,
}

// Echo service handlers
struct EchoUnaryHandler;

#[async_trait]
impl UnaryUnaryHandler<Vec<u8>, Vec<u8>> for EchoUnaryHandler {
    async fn call(
        &self,
        request: Vec<u8>,
        msg_ctx: MessageContext,
        session_ctx: SessionContext,
    ) -> slim_bindings::slimrpc::error::Result<Vec<u8>> {
        let request_str = String::from_utf8_lossy(&request);
        info!(
            "UnaryUnary: Received '{}' from {} in session {}",
            request_str, msg_ctx.source_name, session_ctx.session_id
        );

        let response = format!("Echo unary: {}", request_str);
        Ok(response.into_bytes())
    }
}

struct EchoUnaryStreamHandler;

#[async_trait]
impl UnaryStreamHandler<Vec<u8>, Vec<u8>> for EchoUnaryStreamHandler {
    async fn call(
        &self,
        request: Vec<u8>,
        msg_ctx: MessageContext,
        session_ctx: SessionContext,
    ) -> slim_bindings::slimrpc::error::Result<ResponseStream<Vec<u8>>> {
        let request_str = String::from_utf8_lossy(&request).to_string();
        info!(
            "UnaryStream: Received '{}' from {} in session {}",
            request_str, msg_ctx.source_name, session_ctx.session_id
        );

        // Send 3 responses
        let responses = (0..3).map(move |i| {
            let msg = format!("Echo {} of 3: {}", i + 1, request_str);
            Ok(msg.into_bytes())
        });

        Ok(Box::pin(stream::iter(responses)))
    }
}

struct EchoStreamUnaryHandler;

#[async_trait]
impl StreamUnaryHandler<Vec<u8>, Vec<u8>> for EchoStreamUnaryHandler {
    async fn call(
        &self,
        mut request_stream: RequestStream<Vec<u8>>,
        session_ctx: SessionContext,
    ) -> slim_bindings::slimrpc::error::Result<Vec<u8>> {
        info!(
            "StreamUnary: Receiving stream in session {}",
            session_ctx.session_id
        );

        let mut messages = Vec::new();
        while let Some(result) = request_stream.next().await {
            let (request, msg_ctx) = result?;
            let request_str = String::from_utf8_lossy(&request);
            info!(
                "StreamUnary: Received '{}' from {}",
                request_str, msg_ctx.source_name
            );
            messages.push(request_str.to_string());
        }

        let response = format!(
            "Echo stream-unary: Received {} messages: [{}]",
            messages.len(),
            messages.join(", ")
        );
        Ok(response.into_bytes())
    }
}

struct EchoStreamStreamHandler;

#[async_trait]
impl StreamStreamHandler<Vec<u8>, Vec<u8>> for EchoStreamStreamHandler {
    async fn call(
        &self,
        mut request_stream: RequestStream<Vec<u8>>,
        session_ctx: SessionContext,
    ) -> slim_bindings::slimrpc::error::Result<ResponseStream<Vec<u8>>> {
        info!(
            "StreamStream: Receiving stream in session {} (processing asynchronously)",
            session_ctx.session_id
        );

        // Create channel for responses
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn task to process requests and generate responses asynchronously
        // This demonstrates true bidirectional streaming - processing requests
        // as they arrive and sending responses immediately
        tokio::spawn(async move {
            while let Some(result) = request_stream.next().await {
                match result {
                    Ok((request, msg_ctx)) => {
                        let request_str = String::from_utf8_lossy(&request);
                        info!(
                            "StreamStream: Received '{}' from {} - processing immediately",
                            request_str, msg_ctx.source_name
                        );

                        // Process and send response immediately (async)
                        let response = format!("Echo: {}", request_str);
                        if tx.send(Ok(response.into_bytes())).is_err() {
                            info!("StreamStream: Response channel closed");
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
            info!("StreamStream: Finished processing request stream");
        });

        Ok(Box::pin(
            tokio_stream::wrappers::UnboundedReceiverStream::new(rx),
        ))
    }
}

// Simple echo service
struct SimpleEchoHandler;

#[async_trait]
impl UnaryUnaryHandler<Vec<u8>, Vec<u8>> for SimpleEchoHandler {
    async fn call(
        &self,
        request: Vec<u8>,
        msg_ctx: MessageContext,
        session_ctx: SessionContext,
    ) -> slim_bindings::slimrpc::error::Result<Vec<u8>> {
        let request_str = String::from_utf8_lossy(&request);
        info!(
            "SimpleEcho: '{}' from {} in session {}",
            request_str, msg_ctx.source_name, session_ctx.session_id
        );

        // Simple echo back
        Ok(request)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Create SLIM app configuration and connect (this initializes tracing internally)
    let config =
        RpcAppConfig::with_shared_secret(args.local.clone(), args.endpoint.clone(), args.secret)
            .with_opentelemetry(args.opentelemetry);

    // Create SLIM app and connect
    let RpcAppConnection {
        app,
        connection_id: conn_id,
    } = create_local_app(config)
        .await
        .context("Failed to create local app")?;

    // Now logging is available
    info!("Starting SLIMRpc server");
    info!("Local: {}", args.local);
    info!("Endpoint: {}", args.endpoint);
    info!("Connected to SLIM service with conn_id: {}", conn_id);

    // Create server with the connection ID
    // Create RPC server (using Rust-specific constructor)
    let mut server_owned = RpcServer::new_rust(app, conn_id);

    // Register EchoService handlers
    let mut echo_handlers: HashMap<String, RpcHandler> = HashMap::new();
    echo_handlers.insert(
        "Echo".to_string(),
        RpcHandler::UnaryUnary(Box::new(EchoUnaryHandler)),
    );
    echo_handlers.insert(
        "EchoStream".to_string(),
        RpcHandler::UnaryStream(Box::new(EchoUnaryStreamHandler)),
    );
    echo_handlers.insert(
        "StreamEcho".to_string(),
        RpcHandler::StreamUnary(Box::new(EchoStreamUnaryHandler)),
    );
    echo_handlers.insert(
        "StreamEchoStream".to_string(),
        RpcHandler::StreamStream(Box::new(EchoStreamStreamHandler)),
    );

    server_owned.register_method_handlers("EchoService", echo_handlers);

    // Register simple echo handler
    server_owned.register_rpc(
        "SimpleService",
        "Echo",
        RpcHandler::UnaryUnary(Box::new(SimpleEchoHandler)),
    );

    info!("Server registered handlers:");
    info!("  - /EchoService/Echo (unary-unary)");
    info!("  - /EchoService/EchoStream (unary-stream)");
    info!("  - /EchoService/StreamEcho (stream-unary)");
    info!("  - /EchoService/StreamEchoStream (stream-stream)");
    info!("  - /SimpleService/Echo (unary-unary)");

    // Run server
    info!("Server is running...");

    // For Rust async examples, wrap in Arc and use run_async
    use std::sync::Arc;
    let server = Arc::new(server_owned);
    server.run_async().await?;

    Ok(())
}
