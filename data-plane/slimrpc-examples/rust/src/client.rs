// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod common;

use anyhow::{Context, Result};
use clap::Parser;
use common::{SLIMAppConfig, create_local_app, split_id};
use futures::stream;
use futures::StreamExt;
use slim_bindings::Channel;
use std::time::Duration;
use tracing::{info, Level};

#[derive(Parser, Debug)]
#[command(name = "slimrpc-client")]
#[command(about = "SLIMRpc client example", long_about = None)]
struct Args {
    /// Local identity (org/namespace/app)
    #[arg(short, long, default_value = "agntcy/ns/client")]
    local: String,

    /// Remote service identity (org/namespace/service)
    #[arg(short, long, default_value = "agntcy/ns/server")]
    remote: String,

    /// SLIM endpoint
    #[arg(short, long, default_value = "http://localhost:46357")]
    endpoint: String,

    /// Shared secret for authentication
    #[arg(short, long, default_value = "abcde-12345-fedcb-67890-deadc000")]
    secret: String,

    /// RPC method to call
    #[arg(short, long, default_value = "/EchoService/Echo")]
    method: String,

    /// Message to send
    #[arg(short = 'M', long, default_value = "Hello from client")]
    message: String,

    /// RPC type: unary, unary-stream, stream-unary, stream-stream
    #[arg(short = 't', long, default_value = "unary")]
    rpc_type: String,

    /// Number of iterations for streaming
    #[arg(short, long, default_value = "3")]
    iterations: usize,

    /// Timeout in seconds
    #[arg(long, default_value = "30")]
    timeout: u64,

    /// Enable OpenTelemetry
    #[arg(long)]
    opentelemetry: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let args = Args::parse();

    info!("Starting SLIMRpc client");
    info!("Local: {}", args.local);
    info!("Remote: {}", args.remote);
    info!("Endpoint: {}", args.endpoint);
    info!("Method: {}", args.method);
    info!("RPC Type: {}", args.rpc_type);

    // Create SLIM app configuration
    let config = SLIMAppConfig::with_shared_secret(args.local, args.endpoint, args.secret)
        .with_opentelemetry(args.opentelemetry);

    // Create SLIM app and connect
    let (app, conn_id) = create_local_app(&config).await.context("Failed to create local app")?;

    info!("Connected to SLIM service with conn_id: {}", conn_id);

    // Parse remote identity
    let remote_name = split_id(&args.remote)
        .context("Failed to parse remote identity")?;

    // Create channel with the connection ID
    let channel = Channel::new(remote_name, app, conn_id);

    let timeout = Some(Duration::from_secs(args.timeout));

    // Execute RPC based on type
    match args.rpc_type.as_str() {
        "unary" | "unary-unary" => {
            unary_unary_example(&channel, &args.method, &args.message, timeout).await?;
        }
        "unary-stream" => {
            unary_stream_example(&channel, &args.method, &args.message, timeout).await?;
        }
        "stream-unary" => {
            stream_unary_example(
                &channel,
                &args.method,
                &args.message,
                args.iterations,
                timeout,
            )
            .await?;
        }
        "stream-stream" => {
            stream_stream_example(
                &channel,
                &args.method,
                &args.message,
                args.iterations,
                timeout,
            )
            .await?;
        }
        _ => {
            anyhow::bail!("Unknown RPC type: {}", args.rpc_type);
        }
    }

    info!("Client finished successfully");
    Ok(())
}

async fn unary_unary_example(
    channel: &Channel,
    method: &str,
    message: &str,
    timeout: Option<Duration>,
) -> Result<()> {
    info!("=== Unary-Unary RPC Example ===");

    let request = message.as_bytes().to_vec();

    info!("Sending request: {}", message);
    let response = channel
        .unary_unary(method, request, timeout, None)
        .await
        .context("Unary-unary RPC failed")?;

    let response_str = String::from_utf8_lossy(&response);
    info!("Received response: {}", response_str);

    Ok(())
}

async fn unary_stream_example(
    channel: &Channel,
    method: &str,
    message: &str,
    timeout: Option<Duration>,
) -> Result<()> {
    info!("=== Unary-Stream RPC Example ===");

    let request = message.as_bytes().to_vec();

    info!("Sending request: {}", message);
    let mut response_stream = channel
        .unary_stream(method, request, timeout, None)
        .await
        .context("Unary-stream RPC failed")?;

    info!("Receiving stream responses...");
    let mut count = 0;
    while let Some(response) = response_stream.next().await {
        let response = response.context("Error receiving stream response")?;
        let response_str = String::from_utf8_lossy(&response);
        count += 1;
        info!("Response {}: {}", count, response_str);
    }

    info!("Received {} responses", count);
    Ok(())
}

async fn stream_unary_example(
    channel: &Channel,
    method: &str,
    message: &str,
    iterations: usize,
    timeout: Option<Duration>,
) -> Result<()> {
    info!("=== Stream-Unary RPC Example ===");

    // Create request stream with owned strings
    let message_owned = message.to_string();
    let requests = (0..iterations).map(move |i| {
        let msg = format!("{} - iteration {}", message_owned, i + 1);
        msg.as_bytes().to_vec()
    });
    let request_stream = Box::pin(stream::iter(requests));

    info!("Sending {} requests...", iterations);
    let response = channel
        .stream_unary(method, request_stream, timeout, None)
        .await
        .context("Stream-unary RPC failed")?;

    let response_str = String::from_utf8_lossy(&response);
    info!("Received final response: {}", response_str);

    Ok(())
}

async fn stream_stream_example(
    channel: &Channel,
    method: &str,
    message: &str,
    iterations: usize,
    timeout: Option<Duration>,
) -> Result<()> {
    info!("=== Stream-Stream RPC Example ===");

    // Create request stream with owned strings
    let message_owned = message.to_string();
    let requests = (0..iterations).map(move |i| {
        let msg = format!("{} - iteration {}", message_owned, i + 1);
        msg.as_bytes().to_vec()
    });
    let request_stream = Box::pin(stream::iter(requests));

    info!("Sending {} requests...", iterations);
    let mut response_stream = channel
        .stream_stream(method, request_stream, timeout, None)
        .await
        .context("Stream-stream RPC failed")?;

    info!("Receiving stream responses...");
    let mut count = 0;
    while let Some(response) = response_stream.next().await {
        let response = response.context("Error receiving stream response")?;
        let response_str = String::from_utf8_lossy(&response);
        count += 1;
        info!("Response {}: {}", count, response_str);
    }

    info!("Received {} responses", count);
    Ok(())
}
