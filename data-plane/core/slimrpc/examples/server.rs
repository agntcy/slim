// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agntcy_slimrpc::{
    MessageContext, RPCHandler, RequestStream, ResponseStream, SLIMAppConfig, Server,
    SessionContext, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::Parser;
use futures::stream;
use futures::StreamExt;
use slim_config::component::ComponentBuilder;
use std::collections::HashMap;
use tracing::{info, Level};
use tracing_subscriber;

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
    ) -> agntcy_slimrpc::error::Result<Vec<u8>> {
        let request_str = String::from_utf8_lossy(&request);
        info!(
            "UnaryUnary: Received '{}' from {} in session {}",
            request_str, msg_ctx.source_name, session_ctx.session_id
        );

        let response = format!("Echo: {}", request_str);
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
    ) -> agntcy_slimrpc::error::Result<ResponseStream<Vec<u8>>> {
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
    ) -> agntcy_slimrpc::error::Result<Vec<u8>> {
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
            "Echo: Received {} messages: [{}]",
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
    ) -> agntcy_slimrpc::error::Result<ResponseStream<Vec<u8>>> {
        info!(
            "StreamStream: Receiving stream in session {}",
            session_ctx.session_id
        );

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Some(result) = request_stream.next().await {
                match result {
                    Ok((request, msg_ctx)) => {
                        let request_str = String::from_utf8_lossy(&request);
                        info!(
                            "StreamStream: Received '{}' from {}",
                            request_str, msg_ctx.source_name
                        );

                        let response = format!("Echo: {}", request_str);
                        if tx.send(Ok(response.into_bytes())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
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
    ) -> agntcy_slimrpc::error::Result<Vec<u8>> {
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
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let args = Args::parse();

    info!("Starting SLIMRpc server");
    info!("Local: {}", args.local);
    info!("Endpoint: {}", args.endpoint);

    // Create SLIM app configuration
    let config = SLIMAppConfig::new(args.local, args.endpoint, args.secret)
        .with_opentelemetry(args.opentelemetry);

    // Create SLIM service
    let service = slim_service::Service::builder()
        .build("slimrpc-server".to_string())
        .context("Failed to create service")?;

    // Create local app
    let (app, rx, conn_id) = agntcy_slimrpc::common::create_local_app(&config, &service)
        .await
        .context("Failed to create local app")?;

    info!("Connected to SLIM service with conn_id: {}", conn_id);

    // Create server with the connection ID
    let mut server = Server::new(app, rx, conn_id);

    // Register EchoService handlers
    let mut echo_handlers: HashMap<String, RPCHandler> = HashMap::new();
    echo_handlers.insert(
        "Echo".to_string(),
        RPCHandler::UnaryUnary(Box::new(EchoUnaryHandler)),
    );
    echo_handlers.insert(
        "EchoStream".to_string(),
        RPCHandler::UnaryStream(Box::new(EchoUnaryStreamHandler)),
    );
    echo_handlers.insert(
        "StreamEcho".to_string(),
        RPCHandler::StreamUnary(Box::new(EchoStreamUnaryHandler)),
    );
    echo_handlers.insert(
        "StreamEchoStream".to_string(),
        RPCHandler::StreamStream(Box::new(EchoStreamStreamHandler)),
    );

    server.register_method_handlers("EchoService", echo_handlers);

    // Register simple echo handler
    server.register_rpc(
        "SimpleService",
        "Echo",
        RPCHandler::UnaryUnary(Box::new(SimpleEchoHandler)),
    );

    info!("Server registered handlers:");
    info!("  - /EchoService/Echo (unary-unary)");
    info!("  - /EchoService/EchoStream (unary-stream)");
    info!("  - /EchoService/StreamEcho (stream-unary)");
    info!("  - /EchoService/StreamEchoStream (stream-stream)");
    info!("  - /SimpleService/Echo (unary-unary)");

    // Run server
    info!("Server is running...");
    server.run().await?;

    Ok(())
}
