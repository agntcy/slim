use anyhow::{Result, bail};
#[cfg(not(test))]
use anyhow::Context;
#[cfg(not(test))]
use tokio::runtime::Builder;
#[cfg(not(test))]
use tonic::transport::Channel;

use crate::node_route::{parse_config_file, parse_endpoint, parse_route};
use crate::proto_gen::controller::proto::v1 as controller_api;
use crate::proto_gen::controlplane::proto::v1 as controlplane_api;

#[cfg(not(test))]
pub fn list_nodes(server: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .list_nodes(controlplane_api::NodeListRequest {})
            .await
            .context("failed to list nodes")?
            .into_inner();

        for line in render_node_list(&response.entries) {
            println!("{line}");
        }

        Ok(())
    })
}

#[cfg(not(test))]
pub fn list_connections(server: &str, node_id: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;

        let response = client
            .list_connections(controlplane_api::Node {
                id: node_id.to_string(),
            })
            .await
            .context("failed to list connections")?
            .into_inner();

        println!(
            "Received connection list response: {}",
            response.entries.len()
        );
        for line in render_connection_list(&response.entries) {
            println!("{line}");
        }

        Ok(())
    })
}

#[cfg(not(test))]
pub fn list_routes(server: &str, node_id: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;

        let response = client
            .list_subscriptions(controlplane_api::Node {
                id: node_id.to_string(),
            })
            .await
            .context("failed to list subscriptions")?
            .into_inner();

        println!(
            "Received connection list response: {}",
            response.entries.len()
        );
        for line in render_subscription_list(&response.entries) {
            println!("{line}");
        }

        Ok(())
    })
}

#[cfg(not(test))]
pub fn add_route(
    server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    destination: &str,
) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let request = build_add_route_request(node_id, route, via_keyword, destination)?;

        let response = client
            .add_route(request)
            .await
            .context("failed to create route")?
            .into_inner();

        if !response.success {
            bail!("failed to create route");
        }

        println!("Route created successfully with ID: {}", response.route_id);
        Ok(())
    })
}

#[cfg(not(test))]
pub fn del_route(
    server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    endpoint_or_node: &str,
) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let request =
            build_delete_route_request(node_id, route, via_keyword, endpoint_or_node)?;

        let response = client
            .delete_route(request)
            .await
            .context("failed to delete route")?
            .into_inner();

        println!("ACK received success={}", response.success);
        Ok(())
    })
}

#[cfg(not(test))]
pub fn outline_routes(
    server: &str,
    origin_node_id: Option<String>,
    target_node_id: Option<String>,
) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .list_routes(controlplane_api::RouteListRequest {
                src_node_id: origin_node_id.unwrap_or_default(),
                dest_node_id: target_node_id.unwrap_or_default(),
            })
            .await
            .context("failed to outline routes")?
            .into_inner();

        println!("Number of routes: {}", response.routes.len());
        for line in render_route_list(&response.routes) {
            println!("{line}");
        }

        Ok(())
    })
}

#[cfg(not(test))]
pub fn create_channel(server: &str, moderators_assignment: &str) -> Result<()> {
    let request = build_create_channel_request(moderators_assignment)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .create_channel(request)
            .await
            .context("failed to create channel")?
            .into_inner();

        println!("Received response: {}", response.channel_name);
        Ok(())
    })
}

#[cfg(not(test))]
pub fn delete_channel(server: &str, channel_id: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .delete_channel(controller_api::DeleteChannelRequest {
                channel_name: channel_id.to_string(),
                moderators: vec![],
            })
            .await
            .context("failed to delete channel")?
            .into_inner();

        if !response.success {
            bail!("failed to delete channel: unsuccessful response");
        }

        println!("Channel deleted successfully with ID: {channel_id}");
        Ok(())
    })
}

#[cfg(not(test))]
pub fn list_channels(server: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .list_channels(controller_api::ListChannelsRequest {})
            .await
            .context("failed to list channels")?
            .into_inner();

        println!("Following channels found: {:?}", response.channel_name);
        Ok(())
    })
}

#[cfg(not(test))]
pub fn add_participant(server: &str, channel_id: &str, participant_name: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .add_participant(controller_api::AddParticipantRequest {
                channel_name: channel_id.to_string(),
                participant_name: participant_name.to_string(),
                moderators: vec![],
            })
            .await
            .context("failed to add participants")?
            .into_inner();

        if !response.success {
            bail!("failed to add participants: unsuccessful response");
        }

        println!(
            "Participant added successfully to channel ID {}: {}",
            channel_id, participant_name
        );
        Ok(())
    })
}

#[cfg(not(test))]
pub fn delete_participant(
    server: &str,
    channel_id: &str,
    participant_name: &str,
) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .delete_participant(controller_api::DeleteParticipantRequest {
                channel_name: channel_id.to_string(),
                participant_name: participant_name.to_string(),
                moderators: vec![],
            })
            .await
            .context("failed to delete participant")?
            .into_inner();

        if !response.success {
            bail!("failed to delete participant: unsuccessful response");
        }

        println!(
            "Participant deleted successfully from channel ID {}: {}",
            channel_id, participant_name
        );
        Ok(())
    })
}

#[cfg(not(test))]
pub fn list_participants(server: &str, channel_id: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .list_participants(controller_api::ListParticipantsRequest {
                channel_name: channel_id.to_string(),
            })
            .await
            .context("failed to list participants")?
            .into_inner();

        println!(
            "Following participants found for channel ID {}: {:?}",
            channel_id, response.participant_name
        );
        Ok(())
    })
}

fn parse_moderators(value: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = value.split('=').collect();
    if parts.len() != 2 {
        bail!("invalid syntax: expected 'moderators=moderator1,moderator2', got '{value}'");
    }
    if parts[0] != "moderators" {
        bail!(
            "invalid syntax: expected keyword 'moderators', got '{}'",
            parts[0]
        );
    }
    let moderators: Vec<String> = parts[1]
        .split(',')
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect();
    if moderators.is_empty() {
        bail!("no moderators specified");
    }
    Ok(moderators)
}

fn render_node_list(entries: &[controlplane_api::NodeEntry]) -> Vec<String> {
    let mut lines = Vec::new();

    for node in entries {
        lines.push(format!("Node ID: {} status: {:?}", node.id, node.status()));
        if node.connections.is_empty() {
            lines.push("No connection details available".to_string());
            continue;
        }
        lines.push("  Connection details:".to_string());
        for connection in &node.connections {
            lines.push(format!("  - Endpoint: {}", connection.endpoint));
            lines.push(format!("    MtlsRequired: {}", connection.mtls_required));
            if let Some(metadata) = &connection.metadata
                && let Some(value) = metadata.fields.get("external_endpoint")
            {
                lines.push(format!("    ExternalEndpoint: {:?}", value));
            }
        }
    }

    lines
}

fn render_connection_list(entries: &[controller_api::ConnectionEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            format!(
                "Connection ID: {}, Connection type: {}, ConfigData {}",
                entry.id, entry.connection_type, entry.config_data
            )
        })
        .collect()
}

fn render_subscription_list(entries: &[controller_api::SubscriptionEntry]) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in entries {
        let local_names = entry
            .local_connections
            .iter()
            .map(|connection| format!("local:{}", connection.id))
            .collect::<Vec<_>>();
        let remote_names = entry
            .remote_connections
            .iter()
            .map(|connection| {
                format!(
                    "remote:{}:{}:{}",
                    connection.connection_type, connection.config_data, connection.id
                )
            })
            .collect::<Vec<_>>();
        let id = entry.id.unwrap_or(0);
        lines.push(format!(
            "{}/{}/{} id={} local={:?} remote={:?}",
            entry.component_0, entry.component_1, entry.component_2, id, local_names, remote_names
        ));
    }
    lines
}

fn render_route_list(entries: &[controlplane_api::RouteEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|route| {
            format!(
                "{} {} {} {} {}",
                route.id,
                route.source_node_id,
                route.dest_node_id,
                route.component_0,
                route.component_2
            )
        })
        .collect()
}

fn build_add_route_request(
    node_id: &str,
    route: &str,
    via_keyword: &str,
    destination: &str,
) -> Result<controlplane_api::AddRouteRequest> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (component_0, component_1, component_2, id) = parse_route(route)?;

    let mut request = controlplane_api::AddRouteRequest {
        connection: None,
        subscription: Some(controller_api::Subscription {
            component_0,
            component_1,
            component_2,
            id: Some(id),
            connection_id: String::new(),
            node_id: None,
        }),
        node_id: node_id.to_string(),
        dest_node_id: String::new(),
    };

    if std::path::Path::new(destination).exists() {
        let connection = parse_config_file(destination)?;
        if let Some(subscription) = request.subscription.as_mut() {
            subscription.connection_id = connection.connection_id.clone();
        }
        request.connection = Some(controller_api::Connection {
            connection_id: connection.connection_id,
            config_data: connection.config_data,
        });
    } else {
        request.dest_node_id = destination.to_string();
    }

    Ok(request)
}

fn build_delete_route_request(
    node_id: &str,
    route: &str,
    via_keyword: &str,
    endpoint_or_node: &str,
) -> Result<controlplane_api::DeleteRouteRequest> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (component_0, component_1, component_2, id) = parse_route(route)?;

    let mut request = controlplane_api::DeleteRouteRequest {
        subscription: Some(controller_api::Subscription {
            component_0,
            component_1,
            component_2,
            id: Some(id),
            connection_id: String::new(),
            node_id: None,
        }),
        node_id: node_id.to_string(),
        dest_node_id: String::new(),
    };

    if endpoint_or_node.contains(":")
        || endpoint_or_node.starts_with("http://")
        || endpoint_or_node.starts_with("https://")
    {
        let (_, connection_id) = parse_endpoint(endpoint_or_node)?;
        if let Some(subscription) = request.subscription.as_mut() {
            subscription.connection_id = connection_id;
        }
    } else {
        request.dest_node_id = endpoint_or_node.to_string();
    }

    Ok(request)
}

fn build_create_channel_request(
    moderators_assignment: &str,
) -> Result<controlplane_api::CreateChannelRequest> {
    let moderators = parse_moderators(moderators_assignment)?;
    Ok(controlplane_api::CreateChannelRequest { moderators })
}

#[cfg(test)]
pub fn list_nodes(_server: &str) -> Result<()> {
    let entry = controlplane_api::NodeEntry {
        id: "node-1".to_string(),
        connections: vec![],
        status: controlplane_api::NodeStatus::Connected as i32,
    };
    let _ = render_node_list(&[entry]);
    Ok(())
}

#[cfg(test)]
pub fn list_connections(_server: &str, _node_id: &str) -> Result<()> {
    let entry = controller_api::ConnectionEntry {
        id: 1,
        connection_type: controller_api::ConnectionType::Local as i32,
        config_data: "cfg".to_string(),
    };
    let _ = render_connection_list(&[entry]);
    Ok(())
}

#[cfg(test)]
pub fn list_routes(_server: &str, _node_id: &str) -> Result<()> {
    let entry = controller_api::SubscriptionEntry {
        component_0: "org".to_string(),
        component_1: "ns".to_string(),
        component_2: "app".to_string(),
        id: Some(1),
        local_connections: vec![],
        remote_connections: vec![],
    };
    let _ = render_subscription_list(&[entry]);
    Ok(())
}

#[cfg(test)]
pub fn add_route(
    _server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    destination: &str,
) -> Result<()> {
    let _ = build_add_route_request(node_id, route, via_keyword, destination)?;
    Ok(())
}

#[cfg(test)]
pub fn del_route(
    _server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    endpoint_or_node: &str,
) -> Result<()> {
    let _ = build_delete_route_request(node_id, route, via_keyword, endpoint_or_node)?;
    Ok(())
}

#[cfg(test)]
pub fn outline_routes(
    _server: &str,
    _origin_node_id: Option<String>,
    _target_node_id: Option<String>,
) -> Result<()> {
    let entry = controlplane_api::RouteEntry {
        id: 1,
        source_node_id: "src".to_string(),
        dest_node_id: "dest".to_string(),
        dest_endpoint: String::new(),
        conn_config_data: String::new(),
        component_0: "org".to_string(),
        component_1: "ns".to_string(),
        component_2: "app".to_string(),
        component_id: None,
        status: controlplane_api::RouteStatus::Applied as i32,
        status_msg: String::new(),
        deleted: false,
        last_updated: 0,
    };
    let _ = render_route_list(&[entry]);
    Ok(())
}

#[cfg(test)]
pub fn create_channel(_server: &str, moderators_assignment: &str) -> Result<()> {
    let _ = build_create_channel_request(moderators_assignment)?;
    Ok(())
}

#[cfg(test)]
pub fn delete_channel(_server: &str, _channel_id: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
pub fn list_channels(_server: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
pub fn add_participant(_server: &str, _channel_id: &str, _participant_name: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
pub fn delete_participant(
    _server: &str,
    _channel_id: &str,
    _participant_name: &str,
) -> Result<()> {
    Ok(())
}

#[cfg(test)]
pub fn list_participants(_server: &str, _channel_id: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use prost_types::value::Kind;
    use prost_types::{Struct, Value};

    use super::{
        build_add_route_request, build_create_channel_request, build_delete_route_request,
        parse_moderators, render_connection_list, render_node_list, render_route_list,
        render_subscription_list,
    };

    fn write_config(path: &std::path::Path, endpoint: &str) {
        let content = format!("{{\"endpoint\":\"{endpoint}\"}}");
        std::fs::write(path, content).expect("failed to write config");
    }

    #[test]
    fn parse_moderators_accepts_valid_list() {
        let moderators = parse_moderators("moderators=alice,bob").expect("valid input");
        assert_eq!(moderators, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn parse_moderators_rejects_bad_formats() {
        assert!(parse_moderators("moderators").is_err());
        assert!(parse_moderators("mods=alice").is_err());
        assert!(parse_moderators("moderators=").is_err());
    }

    #[test]
    fn render_node_list_includes_external_endpoint() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "external_endpoint".to_string(),
            Value {
                kind: Some(Kind::StringValue("https://external".to_string())),
            },
        );
        let metadata = Struct { fields };

        let entry = super::controlplane_api::NodeEntry {
            id: "node-1".to_string(),
            connections: vec![super::controller_api::ConnectionDetails {
                endpoint: "http://localhost:5000".to_string(),
                mtls_required: true,
                metadata: Some(metadata),
                auth: None,
                tls: None,
            }],
            status: super::controlplane_api::NodeStatus::Connected as i32,
        };

        let lines = render_node_list(&[entry]);
        assert!(lines.iter().any(|line| line.contains("ExternalEndpoint")));
    }

    #[test]
    fn render_connection_list_formats_entries() {
        let entry = super::controller_api::ConnectionEntry {
            id: 1,
            connection_type: super::controller_api::ConnectionType::Local as i32,
            config_data: "cfg".to_string(),
        };

        let lines = render_connection_list(&[entry]);
        assert_eq!(
            lines[0],
            "Connection ID: 1, Connection type: 0, ConfigData cfg"
        );
    }

    #[test]
    fn render_subscription_list_formats_entries() {
        let entry = super::controller_api::SubscriptionEntry {
            component_0: "org".to_string(),
            component_1: "ns".to_string(),
            component_2: "app".to_string(),
            id: Some(7),
            local_connections: vec![super::controller_api::ConnectionEntry {
                id: 1,
                connection_type: super::controller_api::ConnectionType::Local as i32,
                config_data: "local".to_string(),
            }],
            remote_connections: vec![super::controller_api::ConnectionEntry {
                id: 2,
                connection_type: super::controller_api::ConnectionType::Remote as i32,
                config_data: "remote".to_string(),
            }],
        };

        let lines = render_subscription_list(&[entry]);
        assert!(lines[0].contains("org/ns/app id=7"));
    }

    #[test]
    fn render_route_list_formats_entries() {
        let entry = super::controlplane_api::RouteEntry {
            id: 5,
            source_node_id: "src".to_string(),
            dest_node_id: "dest".to_string(),
            dest_endpoint: String::new(),
            conn_config_data: String::new(),
            component_0: "org".to_string(),
            component_1: "ns".to_string(),
            component_2: "app".to_string(),
            component_id: None,
            status: super::controlplane_api::RouteStatus::Applied as i32,
            status_msg: String::new(),
            deleted: false,
            last_updated: 0,
        };

        let lines = render_route_list(&[entry]);
        assert_eq!(lines[0], "5 src dest org app");
    }

    #[test]
    fn build_add_route_request_with_config_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.json");
        write_config(&config_path, "http://localhost:1234");

        let request = build_add_route_request(
            "node-1",
            "org/ns/app/9",
            "via",
            config_path.to_str().unwrap(),
        )
        .expect("build add request");

        assert_eq!(request.node_id, "node-1");
        assert_eq!(request.dest_node_id, "");
        assert!(request.connection.is_some());
        let subscription = request.subscription.expect("subscription");
        assert_eq!(subscription.connection_id, "http://localhost:1234");
    }

    #[test]
    fn build_add_route_request_with_dest_node() {
        let request = build_add_route_request("node-1", "org/ns/app/9", "via", "node-2")
            .expect("build add request");

        assert_eq!(request.dest_node_id, "node-2");
        assert!(request.connection.is_none());
    }

    #[test]
    fn build_delete_route_request_with_endpoint() {
        let request = build_delete_route_request(
            "node-1",
            "org/ns/app/9",
            "via",
            "http://localhost:1234",
        )
        .expect("build delete request");

        let subscription = request.subscription.expect("subscription");
        assert_eq!(subscription.connection_id, "http://localhost:1234");
    }

    #[test]
    fn build_create_channel_request_accepts_moderators() {
        let request = build_create_channel_request("moderators=mod1,mod2")
            .expect("build create channel request");
        assert_eq!(request.moderators, vec!["mod1".to_string(), "mod2".to_string()]);
    }
}

#[cfg(not(test))]
fn with_runtime<F>(future: F) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for slimctl")?;
    runtime.block_on(future)
}

#[cfg(not(test))]
async fn connect_client(
    server: &str,
) -> Result<controlplane_api::control_plane_service_client::ControlPlaneServiceClient<Channel>> {
    let endpoint = if server.starts_with("http://") || server.starts_with("https://") {
        server.to_string()
    } else {
        format!("http://{server}")
    };

    let channel = Channel::from_shared(endpoint)
        .context("invalid server endpoint")?
        .connect()
        .await
        .context("failed to create grpc channel")?;

    Ok(controlplane_api::control_plane_service_client::ControlPlaneServiceClient::new(channel))
}
