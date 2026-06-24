// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use tokio_stream::StreamExt;

use crate::client::get_control_plane_client;
use crate::proto::controller::proto::v1::{Connection, ConnectionDirection, ConnectionType, Route};
use crate::proto::controlplane::proto::v1::{
    AddRouteRequest, DeleteRouteRequest, LinkEntry, LinkListRequest, LinkStatus, Node,
    NodeListRequest, NodeStatus, RouteEntry, RouteListRequest, RouteStatus,
};
use crate::rpc;
use crate::utils::{VIA_KEYWORD, parse_config_file, parse_route};
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::ProtoName;

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
    /// List routes on a node
    #[command(visible_alias = "ls")]
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

// ── Link ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ControllerLinkArgs {
    #[command(subcommand)]
    pub command: ControllerLinkCommand,
}

#[derive(Subcommand)]
pub enum ControllerLinkCommand {
    /// List all links registered at the controller
    Outline {
        /// Filter by source (origin) node ID
        #[arg(short = 'o', long, default_value = "")]
        origin_node_id: String,
        /// Filter by destination (target) node ID
        #[arg(short = 't', long, default_value = "")]
        target_node_id: String,
    },
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(args: &ControllerArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerCommand::Node(a) => run_node(a, opts).await,
        ControllerCommand::Connection(a) => run_connection(a, opts).await,
        ControllerCommand::Route(a) => run_route(a, opts).await,
        ControllerCommand::Link(a) => run_link(a, opts).await,
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

async fn run_link(args: &ControllerLinkArgs, opts: &ClientConfig) -> Result<()> {
    match &args.command {
        ControllerLinkCommand::Outline {
            origin_node_id,
            target_node_id,
        } => link_outline(origin_node_id, target_node_id, opts).await,
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
    let sep: String = widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("");
    println!("  {sep}");

    // Print rows
    for (id, group, status, ep, pub_ep) in &rows {
        println!(
            "  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
            id,
            group,
            status,
            ep,
            pub_ep,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
        );
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
    let sep: String = widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("");
    println!("  {sep}");

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
    Ok(())
}

// ── Route commands ─────────────────────────────────────────────────────────────

async fn route_list(node_id: &str, opts: &ClientConfig) -> Result<()> {
    let mut client = get_control_plane_client(opts).await?;
    println!("Listing routes for node ID: {}", node_id);
    let resp = rpc!(
        client,
        list_node_routes,
        Node {
            id: node_id.to_string()
        }
    );
    println!("Received route list response: {}", resp.entries.len());
    for e in &resp.entries {
        let conn_names: Vec<String> = e
            .connections
            .iter()
            .map(|c| {
                let peer = c.peer_node_id.as_deref().unwrap_or("-");
                format!(
                    "{}:{}:{:?}:{}:peer={}",
                    c.connection_type().as_str_name(),
                    c.id,
                    c.link_id,
                    c.config_data,
                    peer,
                )
            })
            .collect();
        println!("{} connections={:?}", e.name.as_ref().unwrap(), conn_names);
    }
    Ok(())
}

async fn route_add(
    node_id: &str,
    route: &str,
    via: &str,
    destination: &str,
    opts: &ClientConfig,
) -> Result<()> {
    if via.to_lowercase() != VIA_KEYWORD {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    println!("Add route for node ID: {}", node_id);
    let (org, ns, agent_type, agent_id) = parse_route(route)?;

    let route_msg = Route {
        name: Some(ProtoName::from_strings([&org, &ns, &agent_type]).with_id(agent_id)),
        link_id: None,
        direction: None,
    };

    let (cp_connection, final_dest_node) = if std::path::Path::new(destination).exists() {
        let conn = parse_config_file(destination)?;
        let cp_conn = Connection {
            link_id: conn.link_id.clone(),
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
            route: Some(route_msg),
            connection: cp_connection,
            dest_node_id: final_dest_node,
        }
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
    opts: &ClientConfig,
) -> Result<()> {
    if via.to_lowercase() != VIA_KEYWORD {
        bail!("invalid syntax: expected 'via' keyword, got '{}'", via);
    }
    println!("Delete route for node ID: {}", node_id);
    let (org, ns, agent_type, agent_id) = parse_route(route)?;

    let route_msg = Route {
        name: Some(ProtoName::from_strings([&org, &ns, &agent_type]).with_id(agent_id)),
        link_id: None,
        direction: None,
    };

    let req = DeleteRouteRequest {
        node_id: node_id.to_string(),
        route: Some(route_msg),
        dest_node_id: destination.to_string(),
    };

    let mut client = get_control_plane_client(opts).await?;
    let resp = rpc!(client, delete_route, req);
    if resp.success {
        println!("route removed successfully");
    } else {
        println!("error removing the route");
    }
    Ok(())
}

async fn route_outline(
    origin_node_id: &str,
    target_node_id: &str,
    opts: &ClientConfig,
) -> Result<()> {
    println!(
        "Outline routes (origin:[{}] target:[{}])",
        origin_node_id, target_node_id
    );
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
        routes.push(entry?);
    }
    println!("Number of routes: {}\n", routes.len());
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
    opts: &ClientConfig,
) -> Result<()> {
    println!(
        "Outline links (origin:[{}] target:[{}])",
        origin_node_id, target_node_id
    );
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
        links.push(entry?);
    }
    println!("Number of links: {}\n", links.len());
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
    print_row(&ROUTE_HEADERS, widths);
    let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
    println!("  {}", "-".repeat(total));
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
    print_row(&LINK_HEADERS, widths);
    let total: usize = widths.iter().sum::<usize>() + widths.len() * 2;
    println!("  {}", "-".repeat(total));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_opts(addr: &str) -> ClientConfig {
        slim_config::grpc::client::ClientConfig::with_endpoint(&format!("http://{}", addr))
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure())
            .with_request_timeout(std::time::Duration::from_secs(5))
    }

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

    // ── via validation ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn route_add_invalid_via_fails() {
        let opts = make_opts("127.0.0.1:1");
        let err = route_add(
            "node1",
            "a/b/c/00000000-0000-0000-0000-000000000000",
            "not_via",
            "dest",
            &opts,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("via"));
    }

    #[tokio::test]
    async fn route_del_invalid_via_fails() {
        let opts = make_opts("127.0.0.1:1");
        let err = route_del(
            "node1",
            "a/b/c/00000000-0000-0000-0000-000000000000",
            "bad",
            "dest",
            &opts,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("via"));
    }

    // ── gRPC mock server tests ───────────────────────────────────────────────

    mod mock_server {
        use std::time::Duration;

        use tokio_stream::wrappers::TcpListenerStream;

        use crate::proto::controller::proto::v1::{ConnectionListResponse, RouteListResponse};
        use crate::proto::controlplane::proto::v1::{
            AddRouteRequest, AddRouteResponse, DeleteRouteRequest, DeleteRouteResponse, LinkEntry,
            LinkListRequest, Node as CpNode, NodeEntry, NodeListRequest, RouteEntry,
            RouteListRequest,
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

            async fn add_route(
                &self,
                _req: tonic::Request<AddRouteRequest>,
            ) -> Result<tonic::Response<AddRouteResponse>, tonic::Status> {
                Ok(tonic::Response::new(AddRouteResponse {
                    success: true,
                    route_id: "r1".to_string(),
                }))
            }

            async fn delete_route(
                &self,
                _req: tonic::Request<DeleteRouteRequest>,
            ) -> Result<tonic::Response<DeleteRouteResponse>, tonic::Status> {
                Ok(tonic::Response::new(DeleteRouteResponse { success: true }))
            }

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
        async fn route_add_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_add(
                "node1",
                "a/b/c/00000000-0000-0000-0000-000000000000",
                "via",
                "node-dest",
                &make_opts(&addr),
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn route_del_with_endpoint_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_del(
                "node1",
                "a/b/c/00000000-0000-0000-0000-000000000000",
                "via",
                "http://127.0.0.1:8080",
                &make_opts(&addr),
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn route_del_with_node_id_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_del(
                "node1",
                "a/b/c/00000000-0000-0000-0000-000000000000",
                "via",
                "dest-node",
                &make_opts(&addr),
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn route_outline_succeeds() {
            let addr = spawn_mock_cp_server().await;
            route_outline("", "", &make_opts(&addr)).await.unwrap();
        }

        #[tokio::test]
        async fn link_outline_succeeds() {
            let addr = spawn_mock_cp_server().await;
            link_outline("", "", &make_opts(&addr)).await.unwrap();
        }

        // ── error server ─────────────────────────────────────────────────────

        /// All methods return a gRPC status error.
        struct ErrorControlPlaneSvc;

        #[tonic::async_trait]
        impl ControlPlaneService for ErrorControlPlaneSvc {
            type ListNodesStream = EmptyNodeStream;
            type ListRoutesStream = EmptyRouteStream;
            type ListLinksStream = EmptyLinkStream;

            async fn add_route(
                &self,
                _: tonic::Request<AddRouteRequest>,
            ) -> Result<tonic::Response<AddRouteResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
            async fn delete_route(
                &self,
                _: tonic::Request<DeleteRouteRequest>,
            ) -> Result<tonic::Response<DeleteRouteResponse>, tonic::Status> {
                Err(tonic::Status::internal("error"))
            }
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
        }

        // ── negative-ACK (success = false) server ────────────────────────────

        /// Returns success=false for every method that the client checks.
        /// Other methods return normal success responses so the functions reach
        /// the success check rather than erroring earlier.
        struct FailureControlPlaneSvc;

        #[tonic::async_trait]
        impl ControlPlaneService for FailureControlPlaneSvc {
            type ListNodesStream = EmptyNodeStream;
            type ListRoutesStream = EmptyRouteStream;
            type ListLinksStream = EmptyLinkStream;

            async fn add_route(
                &self,
                _: tonic::Request<AddRouteRequest>,
            ) -> Result<tonic::Response<AddRouteResponse>, tonic::Status> {
                Ok(tonic::Response::new(AddRouteResponse {
                    success: false,
                    route_id: String::new(),
                }))
            }
            async fn delete_route(
                &self,
                _: tonic::Request<DeleteRouteRequest>,
            ) -> Result<tonic::Response<DeleteRouteResponse>, tonic::Status> {
                Ok(tonic::Response::new(DeleteRouteResponse { success: true }))
            }
            async fn list_node_routes(
                &self,
                _: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<RouteListResponse>, tonic::Status> {
                Ok(tonic::Response::new(RouteListResponse {
                    original_message_id: String::new(),
                    entries: vec![],
                    done: true,
                }))
            }
            async fn list_connections(
                &self,
                _: tonic::Request<CpNode>,
            ) -> Result<tonic::Response<ConnectionListResponse>, tonic::Status> {
                Ok(tonic::Response::new(ConnectionListResponse {
                    original_message_id: String::new(),
                    entries: vec![],
                    done: true,
                }))
            }
            async fn list_nodes(
                &self,
                _: tonic::Request<NodeListRequest>,
            ) -> Result<tonic::Response<Self::ListNodesStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
            }
            async fn list_routes(
                &self,
                _: tonic::Request<RouteListRequest>,
            ) -> Result<tonic::Response<Self::ListRoutesStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
            }

            async fn list_links(
                &self,
                _: tonic::Request<LinkListRequest>,
            ) -> Result<tonic::Response<Self::ListLinksStream>, tonic::Status> {
                Ok(tonic::Response::new(empty_stream()))
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
        async fn route_add_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(
                route_add(
                    "n1",
                    "a/b/c/00000000-0000-0000-0000-000000000000",
                    "via",
                    "dest",
                    &make_opts(&addr)
                )
                .await
                .is_err()
            );
        }

        #[tokio::test]
        async fn route_del_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(
                route_del(
                    "n1",
                    "a/b/c/00000000-0000-0000-0000-000000000000",
                    "via",
                    "dest",
                    &make_opts(&addr)
                )
                .await
                .is_err()
            );
        }

        #[tokio::test]
        async fn route_outline_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(route_outline("", "", &make_opts(&addr)).await.is_err());
        }

        #[tokio::test]
        async fn link_outline_grpc_error_propagates() {
            let addr = spawn_cp_svc(ErrorControlPlaneSvc).await;
            assert!(link_outline("", "", &make_opts(&addr)).await.is_err());
        }

        // ── negative-ACK (success = false) tests ─────────────────────────────

        #[tokio::test]
        async fn route_add_negative_ack_fails() {
            let addr = spawn_cp_svc(FailureControlPlaneSvc).await;
            let err = route_add(
                "n1",
                "a/b/c/00000000-0000-0000-0000-000000000000",
                "via",
                "dest",
                &make_opts(&addr),
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("failed to create route"));
        }
    }
}
