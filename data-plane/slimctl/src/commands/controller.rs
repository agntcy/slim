// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::client::get_control_plane_client;
use crate::config::ResolvedOpts;
use crate::proto::controller::proto::v1::{
    AddParticipantRequest, Connection, DeleteChannelRequest, DeleteParticipantRequest,
    ListChannelsRequest, ListParticipantsRequest, Subscription,
};
use crate::proto::controlplane::proto::v1::{
    AddRouteRequest, CreateChannelRequest, DeleteRouteRequest, Node, NodeListRequest, RouteEntry,
    RouteListRequest, RouteStatus,
};
use crate::rpc;
use crate::utils::{is_endpoint, parse_config_file, parse_endpoint, parse_route};

#[derive(Args)]
pub struct ControllerArgs {
    #[command(subcommand)]
    pub command: ControllerCommand,
}

#[derive(Subcommand)]
pub enum ControllerCommand {
    /// Access node information through the control plane
    #[command(aliases = ["n", "nodes", "instance"])]
    Node(ControllerNodeArgs),
    /// Manage SLIM connections via the control plane
    #[command(alias = "conn")]
    Connection(ControllerConnectionArgs),
    /// Manage SLIM routes via the control plane
    Route(ControllerRouteArgs),
    /// Manage SLIM channels (MLS groups)
    Channel(ControllerChannelArgs),
    /// Manage channel participants
    Participant(ControllerParticipantArgs),
}

// ── Node ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerNodeArgs {
    #[command(subcommand)]
    pub command: ControllerNodeCommand,
}

#[derive(Subcommand)]
pub enum ControllerNodeCommand {
    /// List nodes connected to the control plane
    #[command(alias = "ls")]
    List,
}

// ── Connection ────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerConnectionArgs {
    #[command(subcommand)]
    pub command: ControllerConnectionCommand,
}

#[derive(Subcommand)]
pub enum ControllerConnectionCommand {
    /// List active connections on a node
    #[command(alias = "ls")]
    List {
        /// ID of the node
        #[arg(short = 'n', long, required = true)]
        node_id: String,
    },
}

// ── Route ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerRouteArgs {
    #[command(subcommand)]
    pub command: ControllerRouteCommand,
}

#[derive(Subcommand)]
pub enum ControllerRouteCommand {
    /// List subscriptions on a node
    #[command(alias = "ls")]
    List {
        /// ID of the node to manage routes for
        #[arg(short = 'n', long, required = true)]
        node_id: String,
    },
    /// Add a route to a SLIM instance via <slim-node-id or path_to_config.json>
    Add {
        /// ID of the node to manage routes for
        #[arg(short = 'n', long, required = true)]
        node_id: String,
        /// Route in org/namespace/agentname/agentid format
        route: String,
        /// Literal keyword "via"
        via: String,
        /// Destination node ID or path to JSON config file
        destination: String,
    },
    /// Delete a route from a SLIM instance via <slim-node-id or http|https://host:port>
    Del {
        /// ID of the node to manage routes for
        #[arg(short = 'n', long, required = true)]
        node_id: String,
        /// Route in org/namespace/agentname/agentid format
        route: String,
        /// Literal keyword "via"
        via: String,
        /// Destination node ID or endpoint URL
        destination: String,
    },
    /// List all routes registered at the controller
    Outline {
        /// Filter by source (origin) node ID
        #[arg(short = 'o', long, default_value = "")]
        origin_node_id: String,
        /// Filter by destination (target) node ID
        #[arg(short = 't', long, default_value = "")]
        target_node_id: String,
    },
}

// ── Channel ───────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerChannelArgs {
    #[command(subcommand)]
    pub command: ControllerChannelCommand,
}

#[derive(Subcommand)]
pub enum ControllerChannelCommand {
    /// Create a new channel (usage: create moderators=mod1,mod2)
    Create {
        /// Moderators specification: moderators=mod1,mod2
        moderators_param: String,
    },
    /// Delete a channel
    Delete {
        /// Channel name/ID
        channel_name: String,
    },
    /// List channels
    #[command(alias = "ls")]
    List,
}

// ── Participant ───────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerParticipantArgs {
    #[command(subcommand)]
    pub command: ControllerParticipantCommand,
}

#[derive(Subcommand)]
pub enum ControllerParticipantCommand {
    /// Add a participant to a channel
    Add {
        participant_name: String,
        /// ID of the channel
        #[arg(short = 'c', long, required = true)]
        channel_id: String,
    },
    /// Delete a participant from a channel
    Delete {
        participant_name: String,
        /// ID of the channel
        #[arg(short = 'c', long, required = true)]
        channel_id: String,
    },
    /// List participants in a channel
    #[command(alias = "ls")]
    List {
        /// ID of the channel
        #[arg(short = 'c', long, required = true)]
        channel_id: String,
    },
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(args: &ControllerArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerCommand::Node(a) => run_node(a, opts).await,
        ControllerCommand::Connection(a) => run_connection(a, opts).await,
        ControllerCommand::Route(a) => run_route(a, opts).await,
        ControllerCommand::Channel(a) => run_channel(a, opts).await,
        ControllerCommand::Participant(a) => run_participant(a, opts).await,
    }
}

async fn run_node(args: &ControllerNodeArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerNodeCommand::List => node_list(opts).await,
    }
}

async fn run_connection(args: &ControllerConnectionArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerConnectionCommand::List { node_id } => connection_list(node_id, opts).await,
    }
}

async fn run_route(args: &ControllerRouteArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerRouteCommand::List { node_id } => route_list(node_id, opts).await,
        ControllerRouteCommand::Add {
            node_id,
            route,
            via,
            destination,
        } => route_add(node_id, route, via, destination, opts).await,
        ControllerRouteCommand::Del {
            node_id,
            route,
            via,
            destination,
        } => route_del(node_id, route, via, destination, opts).await,
        ControllerRouteCommand::Outline {
            origin_node_id,
            target_node_id,
        } => route_outline(origin_node_id, target_node_id, opts).await,
    }
}

// ── Node commands ──────────────────────────────────────────────────────────────

async fn node_list(opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(client, list_nodes, NodeListRequest {}, opts);
    for node in &resp.entries {
        println!("Node ID: {} status: {:?}", node.id, node.status);
        if !node.connections.is_empty() {
            println!("  Connection details:");
            for conn in &node.connections {
                println!("  - Endpoint: {}", conn.endpoint);
                println!("    MtlsRequired: {}", conn.mtls_required);
                if let Some(meta) = &conn.metadata
                    && let Some(ext) = meta.fields.get("external_endpoint")
                    && let Some(prost_types::value::Kind::StringValue(val)) = &ext.kind
                    && !val.is_empty()
                {
                    println!("    ExternalEndpoint: {}", val);
                }
            }
        } else {
            println!("  No connection details available");
        }
    }
    Ok(())
}

// ── Connection commands ────────────────────────────────────────────────────────

async fn connection_list(node_id: &str, opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    println!("Listing connections for node ID: {}", node_id);
    let resp = rpc!(
        client,
        list_connections,
        Node {
            id: node_id.to_string()
        },
        opts
    );
    println!("Received connection list response: {}", resp.entries.len());
    for entry in &resp.entries {
        println!(
            "Connection ID: {}, Connection type: {:?}, ConfigData: {}",
            entry.id, entry.connection_type, entry.config_data
        );
    }
    Ok(())
}

// ── Route commands ─────────────────────────────────────────────────────────────

async fn route_list(node_id: &str, opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    println!("Listing routes for node ID: {}", node_id);
    let resp = rpc!(
        client,
        list_subscriptions,
        Node {
            id: node_id.to_string()
        },
        opts
    );
    println!(
        "Received subscription list response: {}",
        resp.entries.len()
    );
    for e in &resp.entries {
        let local_names: Vec<String> = e
            .local_connections
            .iter()
            .map(|c| format!("local:{}", c.id))
            .collect();
        let remote_names: Vec<String> = e
            .remote_connections
            .iter()
            .map(|c| format!("remote:{:?}:{}:{}", c.connection_type, c.config_data, c.id))
            .collect();
        println!(
            "{}/{}/{} id={:?} local={:?} remote={:?}",
            e.component_0, e.component_1, e.component_2, e.id, local_names, remote_names
        );
    }
    Ok(())
}

async fn route_add(
    node_id: &str,
    route: &str,
    via: &str,
    destination: &str,
    opts: &ResolvedOpts,
) -> Result<()> {
    if via.to_lowercase() != "via" {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    println!("Add route for node ID: {}", node_id);
    let (org, ns, agent_type, agent_id) = parse_route(route)?;

    let mut subscription = Subscription {
        component_0: org,
        component_1: ns,
        component_2: agent_type,
        id: Some(agent_id),
        connection_id: String::new(),
        node_id: None,
    };

    let (cp_connection, final_dest_node) = if std::path::Path::new(destination).exists() {
        let conn = parse_config_file(destination)?;
        subscription.connection_id = conn.connection_id.clone();
        let cp_conn = Connection {
            connection_id: conn.connection_id.clone(),
            config_data: conn.config_data.clone(),
        };
        (Some(cp_conn), String::new())
    } else {
        (None, destination.to_string())
    };

    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        add_route,
        AddRouteRequest {
            node_id: node_id.to_string(),
            subscription: Some(subscription),
            connection: cp_connection,
            dest_node_id: final_dest_node,
        },
        opts
    );
    if !resp.success {
        bail!("failed to create route");
    }
    println!("Route created successfully with ID: {}", resp.route_id);
    Ok(())
}

async fn route_del(
    node_id: &str,
    route: &str,
    via: &str,
    destination: &str,
    opts: &ResolvedOpts,
) -> Result<()> {
    if via.to_lowercase() != "via" {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    println!("Delete route for node ID: {}", node_id);
    let (org, ns, agent_type, agent_id) = parse_route(route)?;

    let mut subscription = Subscription {
        component_0: org,
        component_1: ns,
        component_2: agent_type,
        id: Some(agent_id),
        connection_id: String::new(),
        node_id: None,
    };

    let mut req = DeleteRouteRequest {
        node_id: node_id.to_string(),
        subscription: Some(subscription.clone()),
        dest_node_id: String::new(),
    };

    if is_endpoint(destination) {
        let (_, conn_id) = parse_endpoint(destination)
            .map_err(|e| anyhow::anyhow!("invalid endpoint '{}': {}", destination, e))?;
        subscription.connection_id = conn_id;
        req.subscription = Some(subscription);
    } else {
        req.dest_node_id = destination.to_string();
    }

    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(client, delete_route, req, opts);
    println!("ACK received success={}", resp.success);
    Ok(())
}

async fn route_outline(
    origin_node_id: &str,
    target_node_id: &str,
    opts: &ResolvedOpts,
) -> Result<()> {
    println!(
        "Outline routes (origin:[{}] target:[{}])",
        origin_node_id, target_node_id
    );
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        list_routes,
        RouteListRequest {
            src_node_id: origin_node_id.to_string(),
            dest_node_id: target_node_id.to_string(),
        },
        opts
    );
    let routes = &resp.routes;
    println!("Number of routes: {}\n", routes.len());
    if !routes.is_empty() {
        let col_widths = compute_route_col_widths(routes);
        print_route_header(&col_widths);
        for route in routes {
            print_route_row(route, &col_widths);
        }
    }
    Ok(())
}

const ROUTE_HEADERS: [&str; 8] = [
    "ID",
    "SOURCE",
    "DEST_NODE",
    "DEST_ENDPOINT",
    "SUBSCRIPTION",
    "STATUS",
    "DELETED",
    "LAST_UPDATED",
];

fn route_cells(r: &RouteEntry) -> [String; 8] {
    [
        r.id.to_string(),
        r.source_node_id.clone(),
        if r.dest_node_id.is_empty() {
            "-"
        } else {
            &r.dest_node_id
        }
        .to_string(),
        if r.dest_endpoint.is_empty() {
            "-"
        } else {
            &r.dest_endpoint
        }
        .to_string(),
        build_subscription_str(r),
        route_status_str(r.status),
        if r.deleted { "Yes" } else { "No" }.to_string(),
        format_unix_timestamp(r.last_updated),
    ]
}

fn print_row<T: AsRef<str>>(cells: &[T], widths: &[usize; 8]) {
    let line: Vec<String> = cells
        .iter()
        .zip(widths.iter())
        .map(|(c, &w)| format!("{:<w$}", c.as_ref()))
        .collect();
    println!("  {}", line.join("  "));
}

fn compute_route_col_widths(routes: &[RouteEntry]) -> [usize; 8] {
    let mut widths = ROUTE_HEADERS.map(|h| h.len());
    for r in routes {
        for (w, cell) in widths.iter_mut().zip(route_cells(r).iter()) {
            *w = (*w).max(cell.len());
        }
    }
    widths
}

fn print_route_header(widths: &[usize; 8]) {
    print_row(&ROUTE_HEADERS, widths);
    let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
    println!("  {}", "-".repeat(total));
}

fn print_route_row(route: &RouteEntry, widths: &[usize; 8]) {
    print_row(&route_cells(route), widths);
}

fn build_subscription_str(route: &RouteEntry) -> String {
    let mut s = format!(
        "{}/{}/{}",
        route.component_0, route.component_1, route.component_2
    );
    if let Some(id) = route.component_id {
        s = format!("{}/{}", s, id);
    }
    s
}

fn route_status_str(status: i32) -> String {
    match RouteStatus::try_from(status) {
        Ok(RouteStatus::Applied) => "APPLIED".to_string(),
        Ok(RouteStatus::Failed) => "FAILED".to_string(),
        _ => "UNKNOWN".to_string(),
    }
}

fn format_unix_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ts.to_string())
}

async fn run_channel(args: &ControllerChannelArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerChannelCommand::Create { moderators_param } => {
            channel_create(moderators_param, opts).await
        }
        ControllerChannelCommand::Delete { channel_name } => {
            channel_delete(channel_name, opts).await
        }
        ControllerChannelCommand::List => channel_list(opts).await,
    }
}

// ── Channel commands ───────────────────────────────────────────────────────────

async fn channel_create(moderators_param: &str, opts: &ResolvedOpts) -> Result<()> {
    let moderators = parse_moderators(moderators_param)?;
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        create_channel,
        CreateChannelRequest { moderators },
        opts
    );
    println!("Received response: {}", resp.channel_name);
    Ok(())
}

async fn channel_delete(channel_name: &str, opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        delete_channel,
        DeleteChannelRequest {
            channel_name: channel_name.to_string(),
            moderators: vec![],
        },
        opts
    );
    if !resp.success {
        bail!("failed to delete channel: unsuccessful response");
    }
    println!("Channel deleted successfully: {}", channel_name);
    Ok(())
}

async fn channel_list(opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(client, list_channels, ListChannelsRequest {}, opts);
    println!("Following channels found: {:?}", resp.channel_name);
    Ok(())
}

fn parse_moderators(param: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = param.splitn(2, '=').collect();
    if parts.len() != 2 {
        bail!(
            "invalid syntax: expected 'moderators=mod1,mod2', got '{}'",
            param
        );
    }
    if parts[0] != "moderators" {
        bail!(
            "invalid syntax: expected keyword 'moderators', got '{}'",
            parts[0]
        );
    }
    let mods: Vec<String> = parts[1]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if mods.is_empty() {
        bail!("no moderators specified");
    }
    Ok(mods)
}

async fn run_participant(args: &ControllerParticipantArgs, opts: &ResolvedOpts) -> Result<()> {
    match &args.command {
        ControllerParticipantCommand::Add {
            participant_name,
            channel_id,
        } => participant_add(participant_name, channel_id, opts).await,
        ControllerParticipantCommand::Delete {
            participant_name,
            channel_id,
        } => participant_delete(participant_name, channel_id, opts).await,
        ControllerParticipantCommand::List { channel_id } => {
            participant_list(channel_id, opts).await
        }
    }
}

// ── Participant commands ───────────────────────────────────────────────────────

async fn participant_add(
    participant_name: &str,
    channel_id: &str,
    opts: &ResolvedOpts,
) -> Result<()> {
    println!(
        "Adding participant to channel {}: {}",
        channel_id, participant_name
    );
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        add_participant,
        AddParticipantRequest {
            channel_name: channel_id.to_string(),
            participant_name: participant_name.to_string(),
            moderators: vec![],
        },
        opts
    );
    if !resp.success {
        bail!("failed to add participants: unsuccessful response");
    }
    println!(
        "Participant added successfully to channel {}: {}",
        channel_id, participant_name
    );
    Ok(())
}

async fn participant_delete(
    participant_name: &str,
    channel_id: &str,
    opts: &ResolvedOpts,
) -> Result<()> {
    println!(
        "Deleting participant from channel {}: {}",
        channel_id, participant_name
    );
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        delete_participant,
        DeleteParticipantRequest {
            channel_name: channel_id.to_string(),
            participant_name: participant_name.to_string(),
            moderators: vec![],
        },
        opts
    );
    if !resp.success {
        bail!("failed to delete participant: unsuccessful response");
    }
    println!(
        "Participant deleted successfully from channel {}: {}",
        channel_id, participant_name
    );
    Ok(())
}

async fn participant_list(channel_id: &str, opts: &ResolvedOpts) -> Result<()> {
    println!("Listing participants for channel ID: {}", channel_id);
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        list_participants,
        ListParticipantsRequest {
            channel_name: channel_id.to_string(),
        },
        opts
    );
    println!(
        "Following participants found for channel {}: {:?}",
        channel_id, resp.participant_name
    );
    Ok(())
}
