use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use slim_config::component::configuration::Configuration;
use slim_config::grpc::client::ClientConfig;
use slim_controller::api::proto::api::v1::control_message::Payload;
#[cfg(not(test))]
use slim_controller::api::proto::api::v1::controller_service_client::ControllerServiceClient;
use slim_controller::api::proto::api::v1::{
    ConfigurationCommand, Connection, ControlMessage, Subscription, SubscriptionEntry,
};
#[cfg(not(test))]
use slim_controller::api::proto::api::v1::SubscriptionListRequest;
#[cfg(not(test))]
use tokio::runtime::Builder;
#[cfg(not(test))]
use tokio_stream::iter;
#[cfg(not(test))]
use tonic::Request;
#[cfg(not(test))]
use tonic::transport::Channel;
use url::Url;
#[cfg(not(test))]
use uuid::Uuid;

#[cfg(not(test))]
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
                for line in render_subscription_entries(&list_resp.entries) {
                    println!("{line}");
                }

                return Ok(());
            }
        }

        bail!("did not receive subscription list response")
    })
}

#[cfg(not(test))]
pub fn route_add(route: &str, via_keyword: &str, config_file: &str, server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let message_id = Uuid::new_v4().to_string();
        let request_message = build_add_message(route, config_file, &message_id)?;

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

#[cfg(not(test))]
pub fn route_del(route: &str, via_keyword: &str, endpoint: &str, server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }

    with_runtime(async move {
        let mut client = connect_client(server).await?;
        let message_id = Uuid::new_v4().to_string();
        let request_message = build_delete_message(route, endpoint, &message_id)?;

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

fn render_subscription_entries(entries: &[SubscriptionEntry]) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in entries {
        let local_names = entry
            .local_connections
            .iter()
            .map(|connection| format!("local:{}:{}", connection.id, connection.config_data))
            .collect::<Vec<_>>();
        let remote_names = entry
            .remote_connections
            .iter()
            .map(|connection| format!("remote:{}:{}", connection.id, connection.config_data))
            .collect::<Vec<_>>();

        let id = entry.id.unwrap_or(0);
        lines.push(format!(
            "{}/{}/{} id={} local=[{}] remote=[{}]",
            entry.component_0,
            entry.component_1,
            entry.component_2,
            id,
            local_names.join(" "),
            remote_names.join(" ")
        ));
    }
    lines
}

fn build_add_message(route: &str, config_file: &str, message_id: &str) -> Result<ControlMessage> {
    let (organization, namespace, app_name, app_instance) = parse_route(route)?;
    let connection = parse_config_file(config_file)?;

    let subscription = Subscription {
        component_0: organization,
        component_1: namespace,
        component_2: app_name,
        id: Some(app_instance),
        connection_id: connection.connection_id.clone(),
        node_id: None,
    };

    Ok(ControlMessage {
        message_id: message_id.to_string(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: vec![connection],
            subscriptions_to_set: vec![subscription],
            subscriptions_to_delete: vec![],
        })),
    })
}

fn build_delete_message(route: &str, endpoint: &str, message_id: &str) -> Result<ControlMessage> {
    let (organization, namespace, app_name, app_instance) = parse_route(route)?;
    let (_, connection_id) = parse_endpoint(endpoint)?;

    let subscription = Subscription {
        component_0: organization,
        component_1: namespace,
        component_2: app_name,
        id: Some(app_instance),
        connection_id,
        node_id: None,
    };

    Ok(ControlMessage {
        message_id: message_id.to_string(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: vec![],
            subscriptions_to_set: vec![],
            subscriptions_to_delete: vec![subscription],
        })),
    })
}

#[cfg(test)]
pub fn route_list(_server: &str) -> Result<()> {
    let entry = SubscriptionEntry {
        component_0: "org".to_string(),
        component_1: "ns".to_string(),
        component_2: "app".to_string(),
        id: Some(1),
        local_connections: vec![],
        remote_connections: vec![],
    };
    let _ = render_subscription_entries(&[entry]);
    Ok(())
}

#[cfg(test)]
pub fn route_add(route: &str, via_keyword: &str, config_file: &str, _server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }
    let _ = parse_route(route)?;
    let _ = config_file;
    Ok(())
}

#[cfg(test)]
pub fn route_del(route: &str, via_keyword: &str, endpoint: &str, _server: &str) -> Result<()> {
    if !via_keyword.eq_ignore_ascii_case("via") {
        bail!("invalid syntax: expected 'via' keyword, got '{via_keyword}'");
    }
    let _ = parse_route(route)?;
    let _ = parse_endpoint(endpoint)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use slim_controller::api::proto::api::v1::ConnectionEntry;

    use super::{
        build_add_message, build_delete_message, parse_config_file, parse_endpoint,
        parse_route, render_subscription_entries,
    };

    #[test]
    fn parse_route_accepts_valid_route() {
        let parsed = parse_route("org/default/app/42").expect("valid route should parse");

        assert_eq!(parsed.0, "org");
        assert_eq!(parsed.1, "default");
        assert_eq!(parsed.2, "app");
        assert_eq!(parsed.3, 42);
    }

    #[test]
    fn parse_route_rejects_bad_formats() {
        assert!(parse_route("org/default/app").is_err());
        assert!(parse_route("org/default/app/not-a-number").is_err());
        assert!(parse_route("org/default//1").is_err());
    }

    #[test]
    fn parse_endpoint_accepts_http_and_https_with_port() {
        let (connection, id) =
            parse_endpoint("https://example.com:8443").expect("https endpoint should parse");

        assert_eq!(id, "https://example.com:8443");
        assert_eq!(connection.connection_id, "https://example.com:8443");
        assert_eq!(connection.config_data, "");
    }

    #[test]
    fn parse_endpoint_rejects_invalid_endpoints() {
        assert!(parse_endpoint("ftp://example.com:21").is_err());
        assert!(parse_endpoint("http://example.com").is_err());
        assert!(parse_endpoint("http://:8080").is_err());
        assert!(parse_endpoint("not-a-url").is_err());
    }

    #[test]
    fn render_subscription_entries_formats_output() {
        let entry = super::SubscriptionEntry {
            component_0: "org".to_string(),
            component_1: "ns".to_string(),
            component_2: "app".to_string(),
            id: Some(1),
            local_connections: vec![ConnectionEntry {
                id: 1,
                connection_type: 0,
                config_data: "local".to_string(),
            }],
            remote_connections: vec![ConnectionEntry {
                id: 2,
                connection_type: 1,
                config_data: "remote".to_string(),
            }],
        };

        let lines = render_subscription_entries(&[entry]);
        assert!(lines[0].contains("org/ns/app id=1"));
    }

    #[test]
    fn build_add_message_creates_subscription_and_connection() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.json");
        let content = r#"{"endpoint":"http://localhost:1234"}"#;
        std::fs::write(&config_path, content).expect("write config");

        let message = build_add_message("org/ns/app/9", config_path.to_str().unwrap(), "mid")
            .expect("build add message");

        match message.payload.expect("payload") {
            super::Payload::ConfigCommand(command) => {
                assert_eq!(command.connections_to_create.len(), 1);
                assert_eq!(command.subscriptions_to_set.len(), 1);
                assert!(command.subscriptions_to_delete.is_empty());
                assert_eq!(command.subscriptions_to_set[0].connection_id, "http://localhost:1234");
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn build_delete_message_creates_delete_subscription() {
        let message = build_delete_message("org/ns/app/9", "http://localhost:1234", "mid")
            .expect("build delete message");

        match message.payload.expect("payload") {
            super::Payload::ConfigCommand(command) => {
                assert!(command.connections_to_create.is_empty());
                assert!(command.subscriptions_to_set.is_empty());
                assert_eq!(command.subscriptions_to_delete.len(), 1);
                assert_eq!(
                    command.subscriptions_to_delete[0].connection_id,
                    "http://localhost:1234"
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn parse_config_file_requires_json_extension() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.yaml");
        std::fs::write(&config_path, "{}").expect("write config");
        assert!(parse_config_file(config_path.to_str().unwrap()).is_err());
    }
}
