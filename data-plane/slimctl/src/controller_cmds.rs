use anyhow::{Context, Result, bail};
use tokio::runtime::Builder;
use tonic::transport::Channel;

use crate::node_route::{parse_config_file, parse_endpoint, parse_route};
use crate::proto_gen::controller::proto::v1 as controller_api;
use crate::proto_gen::controlplane::proto::v1 as controlplane_api;

pub fn list_nodes(server: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .list_nodes(controlplane_api::NodeListRequest {})
            .await
            .context("failed to list nodes")?
            .into_inner();

        for node in response.entries {
            println!("Node ID: {} status: {:?}", node.id, node.status());
            if node.connections.is_empty() {
                println!("No connection details available");
            } else {
                println!("  Connection details:");
                for connection in node.connections {
                    println!("  - Endpoint: {}", connection.endpoint);
                    println!("    MtlsRequired: {}", connection.mtls_required);
                    if let Some(metadata) = connection.metadata
                        && let Some(value) = metadata.fields.get("external_endpoint")
                    {
                        println!("    ExternalEndpoint: {:?}", value);
                    }
                }
            }
        }

        Ok(())
    })
}

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
        for entry in response.entries {
            println!(
                "Connection ID: {}, Connection type: {}, ConfigData {}",
                entry.id, entry.connection_type, entry.config_data
            );
        }

        Ok(())
    })
}

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
        for entry in response.entries {
            let mut local_names = Vec::new();
            let mut remote_names = Vec::new();
            for connection in entry.local_connections {
                local_names.push(format!("local:{}", connection.id));
            }
            for connection in entry.remote_connections {
                remote_names.push(format!(
                    "remote:{}:{}:{}",
                    connection.connection_type, connection.config_data, connection.id
                ));
            }
            let id = entry.id.unwrap_or(0);
            println!(
                "{}/{}/{} id={} local={:?} remote={:?}",
                entry.component_0,
                entry.component_1,
                entry.component_2,
                id,
                local_names,
                remote_names
            );
        }

        Ok(())
    })
}

pub fn add_route(
    server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    destination: &str,
) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (component_0, component_1, component_2, id) = parse_route(route)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;

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

pub fn del_route(
    server: &str,
    node_id: &str,
    route: &str,
    via_keyword: &str,
    endpoint_or_node: &str,
) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (component_0, component_1, component_2, id) = parse_route(route)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;

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

        let response = client
            .delete_route(request)
            .await
            .context("failed to delete route")?
            .into_inner();

        println!("ACK received success={}", response.success);
        Ok(())
    })
}

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
        for route in response.routes {
            println!(
                "{} {} {} {} {}",
                route.id,
                route.source_node_id,
                route.dest_node_id,
                route.component_0,
                route.component_2
            );
        }

        Ok(())
    })
}

pub fn create_channel(server: &str, moderators_assignment: &str) -> Result<()> {
    let moderators = parse_moderators(moderators_assignment)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let response = client
            .create_channel(controlplane_api::CreateChannelRequest { moderators })
            .await
            .context("failed to create channel")?
            .into_inner();

        println!("Received response: {}", response.channel_name);
        Ok(())
    })
}

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

pub fn delete_participant(server: &str, channel_id: &str, participant_name: &str) -> Result<()> {
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
