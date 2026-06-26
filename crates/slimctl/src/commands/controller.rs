// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::{Args, Subcommand};
use tokio_stream::StreamExt;

use crate::client::get_control_plane_client;
use crate::proto::controller::proto::v1::{ConnectionDirection, ConnectionType};
use crate::proto::controlplane::proto::v1::{
    AddSegmentRequest, AddTopologyLinkRequest, LinkEntry, LinkListRequest, LinkStatus, Node,
    NodeListRequest, NodeStatus, RemoveSegmentRequest, RemoveTopologyLinkRequest, RouteEntry,
    RouteListRequest, RouteStatus, SegmentListRequest,
};
use crate::rpc;
use slim_config::grpc::client::ClientConfig;

#[derive(Args)]
pub struct ControllerArgs {
    #[command(subcommand)]
    pub command: ControllerCommand,
}

#[derive(Subcommand)]
pub enum ControllerCommand {
    /// Access node information through the control plane
    #[command(visible_aliases = ["n", "nodes", "instance"])]
    Node(ControllerNodeArgs),
    /// Manage SLIM connections via the control plane
    #[command(visible_alias = "conn")]
    Connection(ControllerConnectionArgs),
    /// Manage SLIM routes via the control plane
    Route(ControllerRouteArgs),
    /// List links from the controller DB
    Link(ControllerLinkArgs),
    /// List groups and their nodes
    Group(ControllerGroupArgs),
    /// List segments (routing domains) and their groups
    Segment(ControllerSegmentArgs),
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
    #[command(visible_alias = "ls")]
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
    #[command(visible_alias = "ls")]
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
    /// List routes. With -n: routes on a specific node. Without -n: all routes at the controller.
    #[command(visible_alias = "ls")]
    List {
        /// Show routes for a specific node (per-node view)
        #[arg(short = 'n', long)]
        node_id: Option<String>,
        /// Filter by source (origin) node ID (controller-wide view only)
        #[arg(short = 'o', long, default_value = "")]
        origin_node_id: String,
        /// Filter by destination (target) node ID (controller-wide view only)
        #[arg(short = 't', long, default_value = "")]
        target_node_id: String,
        /// Show all routes (including pending, failed, deleted). Default shows only applied.
        #[arg(short = 'a', long, default_value_t = false)]
        all: bool,
    },
}

// ── Link ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerLinkArgs {
    #[command(subcommand)]
    pub command: ControllerLinkCommand,
}

#[derive(Subcommand)]
pub enum ControllerLinkCommand {
    /// List all links registered at the controller
    #[command(visible_alias = "ls")]
    List {
        /// Filter by source (origin) node ID
        #[arg(short = 'o', long, default_value = "")]
        origin_node_id: String,
        /// Filter by destination (target) node ID
        #[arg(short = 't', long, default_value = "")]
        target_node_id: String,
        /// Show all links (including pending, failed, deleted). Default shows only applied.
        #[arg(short = 'a', long, default_value_t = false)]
        all: bool,
    },
    /// Add a topology link between two groups (API-managed mode only)
    Add {
        /// First group name
        group_a: String,
        /// Second group name
        group_b: String,
        /// Segment name (defaults to "default")
        #[arg(short, long, default_value = "default")]
        segment: String,
    },
    /// Remove a topology link between two groups (API-managed mode only)
    #[command(visible_alias = "rm")]
    Remove {
        /// First group name
        group_a: String,
        /// Second group name
        group_b: String,
        /// Segment name (defaults to "default")
        #[arg(short, long, default_value = "default")]
        segment: String,
    },
}

// ── Group ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerGroupArgs {
    #[command(subcommand)]
    pub command: ControllerGroupCommand,
}

#[derive(Subcommand)]
pub enum ControllerGroupCommand {
    /// List all groups and their nodes
    #[command(visible_alias = "ls")]
    List,
}

// ── Segment ───────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerSegmentArgs {
    #[command(subcommand)]
    pub command: ControllerSegmentCommand,
}

#[derive(Subcommand)]
pub enum ControllerSegmentCommand {
    /// List all segments (routing domains) and their groups
    #[command(visible_alias = "ls")]
    List,
    /// Add a new segment (API-managed mode only)
    Add {
        /// Segment name
        name: String,
    },
    /// Remove a segment and all its links (API-managed mode only)
    #[command(visible_alias = "rm")]
    Remove {
        /// Segment name
        name: String,
    },
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(args: &ControllerArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerCommand::Node(a) => run_node(a, opts).await,
        ControllerCommand::Connection(a) => run_connection(a, opts).await,
        ControllerCommand::Route(a) => run_route(a, opts).await,
        ControllerCommand::Link(a) => run_link(a, opts).await,
        ControllerCommand::Group(a) => run_group(a, opts).await,
        ControllerCommand::Segment(a) => run_segment(a, opts).await,
    }
}

async fn run_node(args: &ControllerNodeArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerNodeCommand::List => node_list(opts).await,
    }
}

async fn run_connection(args: &ControllerConnectionArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerConnectionCommand::List { node_id } => connection_list(node_id, opts).await,
    }
}

async fn run_route(args: &ControllerRouteArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerRouteCommand::List {
            node_id,
            origin_node_id,
            target_node_id,
            all,
        } => match node_id {
            Some(id) => route_list(id, opts).await,
            None => route_outline(origin_node_id, target_node_id, *all, opts).await,
        },
    }
}

async fn run_link(args: &ControllerLinkArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerLinkCommand::List {
            origin_node_id,
            target_node_id,
            all,
        } => link_outline(origin_node_id, target_node_id, *all, opts).await,
        ControllerLinkCommand::Add {
            group_a,
            group_b,
            segment,
        } => link_add(group_a, group_b, segment, opts).await,
        ControllerLinkCommand::Remove {
            group_a,
            group_b,
            segment,
        } => link_remove(group_a, group_b, segment, opts).await,
    }
}

async fn run_group(args: &ControllerGroupArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerGroupCommand::List => group_list(opts).await,
    }
}

async fn run_segment(args: &ControllerSegmentArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerSegmentCommand::List => segment_list(opts).await,
        ControllerSegmentCommand::Add { name } => segment_add(name, opts).await,
        ControllerSegmentCommand::Remove { name } => segment_remove(name, opts).await,
    }
}

// ── Node commands ──────────────────────────────────────────────────────────────

async fn node_list(opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let mut stream = rpc!(client, list_nodes, NodeListRequest {});
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry?);
    }
    println!("{} node(s) registered\n", entries.len());
    if entries.is_empty() {
        return Ok(());
    }

    // Compute column widths
    let headers = ["NODE_ID", "GROUP", "STATUS", "ENDPOINT", "PUBLIC_ENDPOINT"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let rows: Vec<_> = entries
        .iter()
        .map(|e| {
            let status = match NodeStatus::try_from(e.status) {
                Ok(NodeStatus::Connected) => "Connected",
                Ok(NodeStatus::NotConnected) => "NotConnected",
                _ => "Unknown",
            };
            let endpoint = e
                .connections
                .first()
                .map(|c| c.endpoint.as_str())
                .unwrap_or("-");
            let public_endpoint = e
                .connections
                .first()
                .and_then(|c| c.external_endpoint.as_deref())
                .unwrap_or("-");
            (
                e.id.as_str(),
                e.group.as_str(),
                status,
                endpoint,
                public_endpoint,
            )
        })
        .collect();

    for (id, group, status, ep, pub_ep) in &rows {
        widths[0] = widths[0].max(id.len());
        widths[1] = widths[1].max(group.len());
        widths[2] = widths[2].max(status.len());
        widths[3] = widths[3].max(ep.len());
        widths[4] = widths[4].max(pub_ep.len());
    }

    // Print header
    print_table_header(&headers, &widths);

    // Print rows
    for (id, group, status, ep, pub_ep) in &rows {
        print_row(&[*id, *group, *status, *ep, *pub_ep], &widths);
    }
    Ok(())
}

// ── Connection commands ────────────────────────────────────────────────────────

async fn connection_list(node_id: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        list_connections,
        Node {
            id: node_id.to_string()
        }
    );
    println!(
        "Connections for node: {}\n{} connection(s)\n",
        node_id,
        resp.entries.len()
    );
    if resp.entries.is_empty() {
        return Ok(());
    }

    let headers = ["CONN_ID", "TYPE", "DIRECTION", "ENDPOINT", "LINK_ID"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let rows: Vec<_> = resp
        .entries
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
                "APP"
            } else {
                e.peer_node_id.as_deref().unwrap_or("-")
            };
            let link_id = e.link_id.as_deref().unwrap_or("-");
            (e.id.to_string(), conn_type, direction, endpoint, link_id)
        })
        .collect();

    for (id, ct, dir, ep, lid) in &rows {
        widths[0] = widths[0].max(id.len());
        widths[1] = widths[1].max(ct.len());
        widths[2] = widths[2].max(dir.len());
        widths[3] = widths[3].max(ep.len());
        widths[4] = widths[4].max(lid.len());
    }

    print_table_header(&headers, &widths);

    for (id, ct, dir, ep, lid) in &rows {
        print_row(&[id.as_ref(), *ct, *dir, *ep, *lid], &widths);
    }
    Ok(())
}

// ── Route commands ─────────────────────────────────────────────────────────────

async fn route_list(node_id: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(
        client,
        list_node_routes,
        Node {
            id: node_id.to_string()
        }
    );

    // Flatten: one row per (route, connection) pair.
    let headers = ["ROUTE", "TYPE", "ENDPOINT", "LINK_ID"];
    let mut rows: Vec<[String; 4]> = Vec::new();

    for e in &resp.entries {
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
                rows.push([route_name.clone(), conn_type.into(), endpoint, link_id]);
            }
        }
    }

    println!("Routes for node: {}", node_id);
    println!("{} route(s)\n", resp.entries.len());

    if !rows.is_empty() {
        let mut widths = headers.map(|h| h.len());
        for row in &rows {
            for (w, cell) in widths.iter_mut().zip(row.iter()) {
                *w = (*w).max(cell.len());
            }
        }
        print_row(&headers, &widths);
        let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
        println!("  {}", "-".repeat(total));
        for row in &rows {
            print_row(row, &widths);
        }
    }
    Ok(())
}

async fn route_outline(
    origin_node_id: &str,
    target_node_id: &str,
    show_all: bool,
    opts: &ClientConfig,
) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let mut stream = rpc!(
        client,
        list_routes,
        RouteListRequest {
            src_node_id: origin_node_id.to_string(),
            dest_node_id: target_node_id.to_string(),
        }
    );
    let mut routes = Vec::new();
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        if show_all || entry.status == RouteStatus::Applied as i32 {
            routes.push(entry);
        }
    }
    println!(
        "Routes: {} ({})\n",
        routes.len(),
        if show_all {
            "all statuses"
        } else {
            "applied only"
        }
    );
    if !routes.is_empty() {
        let col_widths = compute_route_col_widths(&routes);
        print_route_header(&col_widths);
        for route in &routes {
            print_route_row(route, &col_widths);
        }
    }
    Ok(())
}

async fn link_outline(
    origin_node_id: &str,
    target_node_id: &str,
    show_all: bool,
    opts: &ClientConfig,
) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let mut stream = rpc!(
        client,
        list_links,
        LinkListRequest {
            src_node_id: origin_node_id.to_string(),
            dest_node_id: target_node_id.to_string(),
        }
    );
    let mut links = Vec::new();
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        if show_all || entry.status == LinkStatus::Applied as i32 {
            links.push(entry);
        }
    }
    println!(
        "Links: {} ({})\n",
        links.len(),
        if show_all {
            "all statuses"
        } else {
            "applied only"
        }
    );
    if !links.is_empty() {
        let col_widths = compute_link_col_widths(&links);
        print_link_header(&col_widths);
        for link in &links {
            print_link_row(link, &col_widths);
        }
    }
    Ok(())
}

const ROUTE_HEADERS: [&str; 7] = [
    "SOURCE",
    "DEST_NODE",
    "ROUTE",
    "STATUS",
    "STATUS_MSG",
    "LAST_UPDATED",
    "LINK_ID",
];

fn route_cells(r: &RouteEntry) -> [String; 7] {
    [
        r.source_node_id.clone(),
        if r.dest_node_id.is_empty() {
            "-"
        } else {
            &r.dest_node_id
        }
        .to_string(),
        build_subscription_str(r),
        route_status_str(r.status),
        if r.status_msg.is_empty() {
            "-".to_string()
        } else {
            r.status_msg.clone()
        },
        format_unix_timestamp(r.last_updated),
        if r.link_id.is_empty() {
            "-".to_string()
        } else {
            r.link_id.clone()
        },
    ]
}

fn print_row<T: AsRef<str>>(cells: &[T], widths: &[usize]) {
    let line: Vec<String> = cells
        .iter()
        .zip(widths.iter())
        .map(|(c, &w)| format!("{:<w$}", c.as_ref()))
        .collect();
    println!("  {}", line.join("  "));
}

fn print_table_header(headers: &[&str], widths: &[usize]) {
    print_row(headers, widths);
    let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
    println!("  {}", "-".repeat(total));
}

fn compute_route_col_widths(routes: &[RouteEntry]) -> [usize; 7] {
    let mut widths = ROUTE_HEADERS.map(|h| h.len());
    for r in routes {
        for (w, cell) in widths.iter_mut().zip(route_cells(r).iter()) {
            *w = (*w).max(cell.len());
        }
    }
    widths
}

fn print_route_header(widths: &[usize; 7]) {
    print_table_header(&ROUTE_HEADERS, widths);
}

fn print_route_row(route: &RouteEntry, widths: &[usize; 7]) {
    print_row(&route_cells(route), widths);
}

const LINK_HEADERS: [&str; 8] = [
    "LINK_ID",
    "SOURCE",
    "DEST_NODE",
    "DEST_ENDPOINT",
    "STATUS",
    "STATUS_MSG",
    "DELETED",
    "LAST_UPDATED",
];

fn link_cells(l: &LinkEntry) -> [String; 8] {
    [
        l.link_id.clone(),
        l.source_node_id.clone(),
        l.dest_node_id.clone(),
        l.dest_endpoint.clone(),
        link_status_str(l.status),
        if l.status_msg.is_empty() {
            "-".to_string()
        } else {
            l.status_msg.clone()
        },
        if l.deleted { "Yes" } else { "No" }.to_string(),
        format_unix_timestamp(l.last_updated),
    ]
}

fn compute_link_col_widths(links: &[LinkEntry]) -> [usize; 8] {
    let mut widths = LINK_HEADERS.map(|h| h.len());
    for l in links {
        for (w, cell) in widths.iter_mut().zip(link_cells(l).iter()) {
            *w = (*w).max(cell.len());
        }
    }
    widths
}

fn print_link_header(widths: &[usize; 8]) {
    print_table_header(&LINK_HEADERS, widths);
}

fn print_link_row(link: &LinkEntry, widths: &[usize; 8]) {
    print_row(&link_cells(link), widths);
}

fn build_subscription_str(route: &RouteEntry) -> String {
    route
        .name
        .as_ref()
        .map_or_else(|| "None".to_string(), |n| format!("{}", n))
}

fn route_status_str(status: i32) -> String {
    match RouteStatus::try_from(status) {
        Ok(RouteStatus::Applied) => "APPLIED".to_string(),
        Ok(RouteStatus::Failed) => "FAILED".to_string(),
        Ok(RouteStatus::Deleted) => "DELETED".to_string(),
        Ok(RouteStatus::Pending) => "PENDING".to_string(),
        _ => "UNKNOWN".to_string(),
    }
}

fn link_status_str(status: i32) -> String {
    match LinkStatus::try_from(status) {
        Ok(LinkStatus::Pending) => "PENDING".to_string(),
        Ok(LinkStatus::Applied) => "APPLIED".to_string(),
        Ok(LinkStatus::Failed) => "FAILED".to_string(),
        _ => "UNKNOWN".to_string(),
    }
}

fn format_unix_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ts.to_string())
}

// ── Group commands ────────────────────────────────────────────────────────────

async fn group_list(opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let mut stream = rpc!(client, list_nodes, NodeListRequest {});
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry?);
    }

    // Group nodes by group name
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for e in &entries {
        let group = if e.group.is_empty() {
            "(none)".to_string()
        } else {
            e.group.clone()
        };
        groups.entry(group).or_default().push(e.id.clone());
    }

    println!("{} group(s)\n", groups.len());
    if groups.is_empty() {
        return Ok(());
    }

    let headers = ["GROUP", "NODES"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let rows: Vec<_> = groups
        .iter()
        .map(|(group, nodes)| {
            let node_list = nodes.join(", ");
            (group.as_str(), node_list)
        })
        .collect();

    for (group, node_list) in &rows {
        widths[0] = widths[0].max(group.len());
        widths[1] = widths[1].max(node_list.len());
    }

    print_table_header(&headers, &widths);

    for (group, nodes) in &rows {
        print_row(&[*group, nodes.as_str()], &widths);
    }
    Ok(())
}

// ── Segment commands ──────────────────────────────────────────────────────────

async fn segment_list(opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(client, list_segments, SegmentListRequest {});

    let segments = resp.segments;
    println!("{} segment(s)\n", segments.len());
    if segments.is_empty() {
        return Ok(());
    }

    let headers = ["SEGMENT", "GROUPS", "LINKS"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let rows: Vec<_> = segments
        .iter()
        .map(|s| {
            let groups = s.groups.join(", ");
            let links: String = s
                .edges
                .iter()
                .map(|e| format!("{}↔{}", e.group_a, e.group_b))
                .collect::<Vec<_>>()
                .join(", ");
            (s.name.as_str(), groups, links)
        })
        .collect();

    for (name, groups, links) in &rows {
        widths[0] = widths[0].max(name.len());
        widths[1] = widths[1].max(groups.len());
        widths[2] = widths[2].max(links.len());
    }

    print_table_header(&headers, &widths);

    for (name, groups, links) in &rows {
        print_row(&[*name, groups.as_str(), links.as_str()], &widths);
    }
    Ok(())
}

async fn segment_add(name: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    rpc!(
        client,
        add_segment,
        AddSegmentRequest {
            name: name.to_string()
        }
    );
    println!("Segment '{}' added", name);
    Ok(())
}

async fn segment_remove(name: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    rpc!(
        client,
        remove_segment,
        RemoveSegmentRequest {
            name: name.to_string()
        }
    );
    println!("Segment '{}' removed", name);
    Ok(())
}

async fn link_add(group_a: &str, group_b: &str, segment: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    rpc!(
        client,
        add_topology_link,
        AddTopologyLinkRequest {
            group_a: group_a.to_string(),
            group_b: group_b.to_string(),
            segment: segment.to_string(),
        }
    );
    if segment == "default" {
        println!("Link {}↔{} added", group_a, group_b);
    } else {
        println!(
            "Link {}↔{} added in segment '{}'",
            group_a, group_b, segment
        );
    }
    Ok(())
}

async fn link_remove(
    group_a: &str,
    group_b: &str,
    segment: &str,
    opts: &ClientConfig,
) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    rpc!(
        client,
        remove_topology_link,
        RemoveTopologyLinkRequest {
            group_a: group_a.to_string(),
            group_b: group_b.to_string(),
            segment: segment.to_string(),
        }
    );
    if segment == "default" {
        println!("Link {}↔{} removed", group_a, group_b);
    } else {
        println!(
            "Link {}↔{} removed from segment '{}'",
            group_a, group_b, segment
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::api::ProtoName;

    #[allow(clippy::too_many_arguments)]
    fn make_route(
        source: &str,
        dest_node: &str,
        name: ProtoName,
        status: i32,
        last_updated: i64,
    ) -> RouteEntry {
        RouteEntry {
            id: 1.to_string(),
            source_node_id: source.to_string(),
            dest_node_id: dest_node.to_string(),
            name: Some(name),
            status,
            last_updated,
            ..Default::default()
        }
    }

    // ── route_status_str ────────────────────────────────────────────────────

    #[test]
    fn route_status_applied() {
        assert_eq!(route_status_str(RouteStatus::Applied as i32), "APPLIED");
    }

    #[test]
    fn route_status_failed() {
        assert_eq!(route_status_str(RouteStatus::Failed as i32), "FAILED");
    }

    #[test]
    fn route_status_unspecified_is_unknown() {
        assert_eq!(route_status_str(0), "UNKNOWN");
    }

    #[test]
    fn route_status_pending() {
        assert_eq!(route_status_str(RouteStatus::Pending as i32), "PENDING");
    }

    #[test]
    fn route_status_out_of_range_is_unknown() {
        assert_eq!(route_status_str(999), "UNKNOWN");
    }

    // ── format_unix_timestamp ───────────────────────────────────────────────

    #[test]
    fn format_unix_timestamp_epoch() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_unix_timestamp_known_value() {
        // 2024-02-01T00:00:00Z
        assert_eq!(format_unix_timestamp(1706745600), "2024-02-01T00:00:00Z");
    }

    #[test]
    fn format_unix_timestamp_negative_falls_back_to_raw() {
        // Sufficiently-out-of-range negative value returns the raw number
        let ts = -99_999_999_999_i64;
        let result = format_unix_timestamp(ts);
        assert!(!result.is_empty());
        // Either a valid formatted date or the raw number as string
        assert!(result == ts.to_string() || result.contains('-'));
    }

    // ── build_subscription_str ──────────────────────────────────────────────

    #[test]
    fn build_subscription_str_with_component_id() {
        let name = ProtoName::from_strings(["org", "ns", "agent"]).with_id(2);
        let r = make_route("", "", name, 0, 0);
        assert_eq!(
            build_subscription_str(&r),
            "org/ns/agent/00000000-0000-0000-0000-000000000002"
        );
    }

    #[test]
    fn build_subscription_str_without_component_id() {
        let name = ProtoName::from_strings(["org", "ns", "agent"]);
        let r = make_route("", "", name, 0, 0);
        assert_eq!(build_subscription_str(&r), "org/ns/agent/NULL_COMPONENT");
    }

    #[test]
    fn build_subscription_str_zero_component_id() {
        let name = ProtoName::from_strings(["a", "b", "c"]).with_id(0);
        let r = make_route("", "", name, 0, 0);
        assert_eq!(
            build_subscription_str(&r),
            "a/b/c/00000000-0000-0000-0000-000000000000"
        );
    }

    // ── route_cells ─────────────────────────────────────────────────────────

    #[test]
    fn route_cells_populates_all_columns() {
        let mut r = make_route(
            "src-node",
            "dst-node",
            ProtoName::from_strings(["org", "ns", "agent"]).with_id(7),
            RouteStatus::Applied as i32,
            0,
        );
        r.status_msg = "apply succeeded".to_string();
        let cells = route_cells(&r);
        assert_eq!(cells[0], "src-node"); // source
        assert_eq!(cells[1], "dst-node"); // dest node
        assert_eq!(
            cells[2],
            "org/ns/agent/00000000-0000-0000-0000-000000000007"
        ); // route
        assert_eq!(cells[3], "APPLIED"); // status
        assert_eq!(cells[4], "apply succeeded"); // status msg
    }

    #[test]
    fn route_cells_empty_dest_node_becomes_dash() {
        let name = ProtoName::from_strings(["o", "n", "a"]);
        let r = make_route("src", "", name, 0, 0);
        assert_eq!(route_cells(&r)[1], "-");
    }

    #[test]
    fn route_cells_subscription_column() {
        let name = ProtoName::from_strings(["o", "n", "a"]);
        let r = make_route("src", "", name, 0, 0);
        assert_eq!(route_cells(&r)[2], "o/n/a/NULL_COMPONENT");
    }

    // ── compute_route_col_widths ─────────────────────────────────────────────

    #[test]
    fn col_widths_empty_routes_equals_header_widths() {
        let widths = compute_route_col_widths(&[]);
        for (i, &header) in ROUTE_HEADERS.iter().enumerate() {
            assert_eq!(widths[i], header.len(), "column {i} mismatch");
        }
    }

    #[test]
    fn col_widths_expands_for_long_data() {
        let long_source = "a-very-long-source-node-id-exceeding-header";
        let r = RouteEntry {
            source_node_id: long_source.to_string(),
            ..Default::default()
        };
        let widths = compute_route_col_widths(&[r]);
        assert!(widths[0] >= long_source.len());
    }

    #[test]
    fn col_widths_multiple_routes_takes_max() {
        let r1 = RouteEntry {
            source_node_id: "short".to_string(),
            ..Default::default()
        };
        let r2 = RouteEntry {
            source_node_id: "this-is-a-much-longer-source-node-id".to_string(),
            ..Default::default()
        };
        let widths = compute_route_col_widths(&[r1, r2]);
        assert!(widths[0] >= "this-is-a-much-longer-source-node-id".len());
    }

    // ── print helpers ────────────────────────────────────────────────────────

    #[test]
    fn print_row_no_panic() {
        let cells = ["a", "b", "c", "d", "e", "f", "g"];
        let widths = [5usize; 7];
        print_row(&cells, &widths);
    }

    #[test]
    fn print_route_header_no_panic() {
        let widths = compute_route_col_widths(&[]);
        print_route_header(&widths);
    }

    #[test]
    fn print_route_row_no_panic() {
        let name = ProtoName::from_strings(["org", "ns", "agent"]).with_id(1);
        let r = make_route("src", "dst", name, RouteStatus::Applied as i32, 0);
        let widths = compute_route_col_widths(std::slice::from_ref(&r));
        print_route_row(&r, &widths);
    }

    // ── gRPC mock server tests ───────────────────────────────────────────────

    mod mock_server {
        use std::time::Duration;

        use tokio_stream::wrappers::TcpListenerStream;

        use crate::proto::controller::proto::v1::{ConnectionListResponse, RouteListResponse};
        use crate::proto::controlplane::proto::v1::{
            AddSegmentRequest, AddSegmentResponse, AddTopologyLinkRequest, AddTopologyLinkResponse,
            LinkEntry, LinkListRequest, Node as CpNode, NodeEntry, NodeListRequest,
            RemoveSegmentRequest, RemoveSegmentResponse, RemoveTopologyLinkRequest,
            RemoveTopologyLinkResponse, RouteEntry, RouteListRequest, SegmentListRequest,
            SegmentListResponse,
            control_plane_service_server::{ControlPlaneService, ControlPlaneServiceServer},
        };
        use slim_config::grpc::client::ClientConfig;

        use super::super::*;

        type EmptyNodeStream =
            tokio_stream::wrappers::ReceiverStream<Result<NodeEntry, tonic::Status>>;
        type EmptyRouteStream =
            tokio_stream::wrappers::ReceiverStream<Result<RouteEntry, tonic::Status>>;
        type EmptyLinkStream =
            tokio_stream::wrappers::ReceiverStream<Result<LinkEntry, tonic::Status>>;

        fn empty_stream<T>() -> tokio_stream::wrappers::ReceiverStream<Result<T, tonic::Status>> {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            tokio_stream::wrappers::ReceiverStream::new(rx)
        }

        struct MockControlPlaneSvc;

        #[tonic::async_trait]
        impl ControlPlaneService for MockControlPlaneSvc {
            type ListNodesStream = EmptyNodeStream;
            type ListRoutesStream = EmptyRouteStream;
            type ListLinksStream = EmptyLinkStream;

            async fn list_node_routes(
                &self,
                _req: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<RouteListResponse>, tonic::Status> {
                Ok(tonic::Response::new(RouteListResponse {
                    original_message_id: String::new(),
                    entries: vec![],
                    done: true,
                }))
            }

            async fn list_connections(
                &self,
                _req: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<ConnectionListResponse>, tonic::Status> {
                Ok(tonic::Response::new(ConnectionListResponse {
                    original_message_id: String::new(),
                    entries: vec![],
                    done: true,
                }))
            }

            async fn list_nodes(
                &self,
                _req: tonic::Request<NodeListRequest>,
            ) -> Result<tonic::Response<Self::ListNodesStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
            }

            async fn list_routes(
                &self,
                _req: tonic::Request<RouteListRequest>,
            ) -> Result<tonic::Response<Self::ListRoutesStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
            }

            async fn list_links(
                &self,
                _req: tonic::Request<LinkListRequest>,
            ) -> Result<tonic::Response<Self::ListLinksStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
            }

            async fn list_segments(
                &self,
                _req: tonic::Request<SegmentListRequest>,
            ) -> Result<tonic::Response<SegmentListResponse>, tonic::Status> {
                Ok(tonic::Response::new(SegmentListResponse {
                    segments: vec![],
                }))
            }

            async fn add_segment(
                &self,
                _req: tonic::Request<AddSegmentRequest>,
            ) -> Result<tonic::Response<AddSegmentResponse>, tonic::Status> {
                Ok(tonic::Response::new(AddSegmentResponse {}))
            }

            async fn remove_segment(
                &self,
                _req: tonic::Request<RemoveSegmentRequest>,
            ) -> Result<tonic::Response<RemoveSegmentResponse>, tonic::Status> {
                Ok(tonic::Response::new(RemoveSegmentResponse {}))
            }

            async fn add_topology_link(
                &self,
                _req: tonic::Request<AddTopologyLinkRequest>,
            ) -> Result<tonic::Response<AddTopologyLinkResponse>, tonic::Status> {
                Ok(tonic::Response::new(AddTopologyLinkResponse {}))
            }

            async fn remove_topology_link(
                &self,
                _req: tonic::Request<RemoveTopologyLinkRequest>,
            ) -> Result<tonic::Response<RemoveTopologyLinkResponse>, tonic::Status> {
                Ok(tonic::Response::new(RemoveTopologyLinkResponse {}))
            }
        }

        async fn spawn_mock_cp_server() -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let incoming = TcpListenerStream::new(listener);
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(ControlPlaneServiceServer::new(MockControlPlaneSvc))
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
        async fn node_list_succeeds() {
            let addr = spawn_mock_cp_server().await;
            node_list(&make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn connection_list_succeeds() {
            let addr = spawn_mock_cp_server().await;
            connection_list("node1", &make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn route_list_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_list("node1", &make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn route_outline_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_outline("", "", true, &make_opts(&addr))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn link_outline_succeeds() {
            let addr = spawn_mock_cp_server().await;
            link_outline("", "", true, &make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn group_list_succeeds() {
            let addr = spawn_mock_cp_server().await;
            group_list(&make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn segment_list_succeeds() {
            let addr = spawn_mock_cp_server().await;
            segment_list(&make_opts(&addr)).await.unwrap();
        }

        // ── error server ─────────────────────────────────────────────────────

        /// All methods return a gRPC status error.
        struct ErrorControlPlaneSvc;

        #[tonic::async_trait]
        impl ControlPlaneService for ErrorControlPlaneSvc {
            type ListNodesStream = EmptyNodeStream;
            type ListRoutesStream = EmptyRouteStream;
            type ListLinksStream = EmptyLinkStream;

            async fn list_node_routes(
                &self,
                _: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<RouteListResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
            async fn list_connections(
                &self,
                _: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<ConnectionListResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
            async fn list_nodes(
                &self,
                _: tonic::Request<NodeListRequest>,
            ) -> Result<tonic::Response<Self::ListNodesStream>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
            async fn list_routes(
                &self,
                _: tonic::Request<RouteListRequest>,
            ) -> Result<tonic::Response<Self::ListRoutesStream>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn list_links(
                &self,
                _: tonic::Request<LinkListRequest>,
            ) -> Result<tonic::Response<Self::ListLinksStream>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn list_segments(
                &self,
                _: tonic::Request<SegmentListRequest>,
            ) -> Result<tonic::Response<SegmentListResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn add_segment(
                &self,
                _: tonic::Request<AddSegmentRequest>,
            ) -> Result<tonic::Response<AddSegmentResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn remove_segment(
                &self,
                _: tonic::Request<RemoveSegmentRequest>,
            ) -> Result<tonic::Response<RemoveSegmentResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn add_topology_link(
                &self,
                _: tonic::Request<AddTopologyLinkRequest>,
            ) -> Result<tonic::Response<AddTopologyLinkResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }

            async fn remove_topology_link(
                &self,
                _: tonic::Request<RemoveTopologyLinkRequest>,
            ) -> Result<tonic::Response<RemoveTopologyLinkResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
        }

        async fn spawn_cp_svc<S>(svc: S) -> String
        where
            S: ControlPlaneService,
        {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let incoming = TcpListenerStream::new(listener);
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(ControlPlaneServiceServer::new(svc))
                    .serve_with_incoming(incoming)
                    .await
                    .unwrap();
            });
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            format!("{}:{}", addr.ip(), addr.port())
        }

        // ── gRPC-level error tests ────────────────────────────────────────────

        #[tokio::test]
        async fn node_list_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(node_list(&make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn connection_list_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(connection_list("n1", &make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn route_list_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(route_list("n1", &make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn route_outline_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(
                route_outline("", "", true, &make_opts(&addr))
                    .await
                    .is_err()
            );
        }

        #[tokio::test]
        async fn link_outline_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(link_outline("", "", true, &make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn group_list_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(group_list(&make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn segment_list_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(segment_list(&make_opts(&addr)).await.is_err());
        }
    }
}
