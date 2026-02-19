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
use crate::utils::{parse_config_file, parse_endpoint, parse_route};

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
    if via.to_lowercase() != "via" {
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
                print!("ACK received for {}", a.original_message_id);
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
                        println!("subscription successfully applied: {:?}", ss.subscription);
                    } else {
                        println!(
                            "failed to set subscription {:?}: {}",
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
    if via.to_lowercase() != "via" {
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
                print!("ACK received for {}", a.original_message_id);
                for ss in &a.subscriptions_status {
                    if ss.success {
                        println!("subscription successfully deleted: {:?}", ss.subscription);
                    } else {
                        println!(
                            "failed to delete subscription {:?}: {}",
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
