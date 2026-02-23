use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use slim_config::component::configuration::Configuration;
use slim_config::grpc::client::ClientConfig;
use slim_controller::api::proto::api::v1::control_message::Payload;
use slim_controller::api::proto::api::v1::controller_service_client::ControllerServiceClient;
use slim_controller::api::proto::api::v1::{
    ConfigurationCommand, Connection, ControlMessage, Subscription, SubscriptionListRequest,
};
use tokio::runtime::Builder;
use tokio_stream::iter;
use tonic::Request;
use tonic::transport::Channel;
use url::Url;
use uuid::Uuid;

pub fn route_list(server: &str) -> Result<()> {
    with_runtime(async move {
        let mut client = connect_client(server).await?;

        let request_message = ControlMessage {
            message_id: Uuid::new_v4().to_string(),
            payload: Some(Payload::SubscriptionListRequest(SubscriptionListRequest {})),
        };

        let response = client
            .open_control_channel(Request::new(iter(vec![request_message])))
            .await
            .context("failed to open control channel")?;

        let mut stream = response.into_inner();

        while let Some(message) = stream
            .message()
            .await
            .context("failed to receive stream message")?
        {
            if let Some(Payload::SubscriptionListResponse(list_resp)) = message.payload {
                for entry in list_resp.entries {
                    let local_names = entry
                        .local_connections
                        .iter()
                        .map(|connection| {
                            format!("local:{}:{}", connection.id, connection.config_data)
                        })
                        .collect::<Vec<_>>();
                    let remote_names = entry
                        .remote_connections
                        .iter()
                        .map(|connection| {
                            format!("remote:{}:{}", connection.id, connection.config_data)
                        })
                        .collect::<Vec<_>>();

                    let id = entry.id.unwrap_or(0);
                    println!(
                        "{}/{}/{} id={} local=[{}] remote=[{}]",
                        entry.component_0,
                        entry.component_1,
                        entry.component_2,
                        id,
                        local_names.join(" "),
                        remote_names.join(" ")
                    );
                }

                return Ok(());
            }
        }

        bail!("did not receive subscription list response")
    })
}

pub fn route_add(route: &str, via_keyword: &str, config_file: &str, server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (organization, namespace, app_name, app_instance) = parse_route(route)?;
    let connection = parse_config_file(config_file)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;

        let subscription = Subscription {
            component_0: organization,
            component_1: namespace,
            component_2: app_name,
            id: Some(app_instance),
            connection_id: connection.connection_id.clone(),
            node_id: None,
        };

        let message_id = Uuid::new_v4().to_string();
        let request_message = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::ConfigCommand(ConfigurationCommand {
                connections_to_create: vec![connection],
                subscriptions_to_set: vec![subscription],
                subscriptions_to_delete: vec![],
            })),
        };

        let response = client
            .open_control_channel(Request::new(iter(vec![request_message])))
            .await
            .context("failed to open control channel")?;

        let mut stream = response.into_inner();

        while let Some(message) = stream
            .message()
            .await
            .context("failed to receive stream message")?
        {
            if let Some(Payload::ConfigCommandAck(ack)) = message.payload {
                println!("ACK received for {}", ack.original_message_id);
                for status in ack.connections_status {
                    if status.success {
                        println!("connection successfully applied: {}", status.connection_id);
                    } else {
                        println!(
                            "failed to create connection {}: {}",
                            status.connection_id, status.error_msg
                        );
                    }
                }
                for status in ack.subscriptions_status {
                    if status.success {
                        println!(
                            "subscription successfully applied: {:?}",
                            status.subscription
                        );
                    } else {
                        println!(
                            "failed to set subscription {:?}: {}",
                            status.subscription, status.error_msg
                        );
                    }
                }
                return Ok(());
            }
        }

        bail!("unexpected response type received (not an ACK): {message_id}")
    })
}

pub fn route_del(route: &str, via_keyword: &str, endpoint: &str, server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    let (organization, namespace, app_name, app_instance) = parse_route(route)?;
    let (_, connection_id) = parse_endpoint(endpoint)?;

    with_runtime(async move {
        let mut client = connect_client(server).await?;

        let subscription = Subscription {
            component_0: organization,
            component_1: namespace,
            component_2: app_name,
            id: Some(app_instance),
            connection_id,
            node_id: None,
        };

        let message_id = Uuid::new_v4().to_string();
        let request_message = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::ConfigCommand(ConfigurationCommand {
                connections_to_create: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![subscription],
            })),
        };

        let response = client
            .open_control_channel(Request::new(iter(vec![request_message])))
            .await
            .context("failed to open control channel")?;

        let mut stream = response.into_inner();

        while let Some(message) = stream
            .message()
            .await
            .context("failed to receive stream message")?
        {
            if let Some(Payload::ConfigCommandAck(ack)) = message.payload {
                println!("ACK received for {}", ack.original_message_id);
                for status in ack.subscriptions_status {
                    if status.success {
                        println!(
                            "subscription successfully deleted: {:?}",
                            status.subscription
                        );
                    } else {
                        println!(
                            "failed to delete subscription {:?}: {}",
                            status.subscription, status.error_msg
                        );
                    }
                }
                return Ok(());
            }
        }

        bail!("unexpected response type received (not an ACK): {message_id}")
    })
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
) -> Result<ControllerServiceClient<tonic::transport::Channel>> {
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

    Ok(ControllerServiceClient::new(channel))
}

pub fn parse_route(route: &str) -> Result<(String, String, String, u64)> {
    let parts: Vec<&str> = route.split('/').collect();

    if parts.len() != 4 || parts.iter().any(|part| part.is_empty()) {
        bail!("invalid route format '{route}', expected 'company/namespace/appname/appinstance'");
    }

    let app_instance = parts[3]
        .parse::<u64>()
        .with_context(|| format!("invalid app instance ID (must be u64) {}", parts[3]))?;

    Ok((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
        app_instance,
    ))
}

pub fn parse_endpoint(endpoint: &str) -> Result<(Connection, String)> {
    let parsed =
        Url::parse(endpoint).with_context(|| format!("failed to parse endpoint '{endpoint}'"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => bail!(
            "unsupported scheme '{scheme}' in endpoint '{endpoint}', must be 'http' or 'https'"
        ),
    }

    if parsed.host_str().is_none() {
        bail!("invalid endpoint format '{endpoint}': host part is missing");
    }

    if parsed.port().is_none() {
        bail!("invalid endpoint format '{endpoint}': port part is missing");
    }

    let connection = Connection {
        connection_id: endpoint.to_string(),
        config_data: String::new(),
    };

    Ok((connection, endpoint.to_string()))
}

pub fn parse_config_file(config_file: &str) -> Result<Connection> {
    if config_file.is_empty() {
        bail!("config file path cannot be empty");
    }

    if Path::new(config_file)
        .extension()
        .and_then(|ext| ext.to_str())
        != Some("json")
    {
        bail!("config file '{config_file}' must be a JSON file");
    }

    let data = std::fs::read_to_string(config_file)
        .with_context(|| format!("failed to read config file: {config_file}"))?;

    let parsed_client: ClientConfig = serde_json::from_str(&data)
        .with_context(|| format!("invalid JSON in config file: {config_file}"))?;
    parsed_client
        .validate()
        .map_err(|error| anyhow::anyhow!("failed to validate config data: {error}"))?;

    let json_obj: Value = serde_json::from_str(&data)
        .with_context(|| format!("invalid JSON in config file: {config_file}"))?;

    let endpoint = json_obj
        .get("endpoint")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("'endpoint' key not found in config data"))?;

    Ok(Connection {
        connection_id: endpoint.to_string(),
        config_data: data,
    })
}
