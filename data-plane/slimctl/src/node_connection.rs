use anyhow::Result;
#[cfg(not(test))]
use anyhow::{Context, bail};
#[cfg(not(test))]
use slim_controller::api::proto::api::v1::control_message::Payload;
#[cfg(not(test))]
use slim_controller::api::proto::api::v1::controller_service_client::ControllerServiceClient;
#[cfg(not(test))]
use slim_controller::api::proto::api::v1::{ConnectionListRequest, ControlMessage};
#[cfg(not(test))]
use tokio::runtime::Builder;
#[cfg(not(test))]
use tokio_stream::iter;
#[cfg(not(test))]
use tonic::Request;
#[cfg(not(test))]
use tonic::transport::Channel;
#[cfg(not(test))]
use uuid::Uuid;

#[cfg(not(test))]
pub fn connection_list(server: &str) -> Result<()> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for slimctl")?;

    runtime.block_on(async move {
        let mut client = connect_client(server).await?;

        let request_message = ControlMessage {
            message_id: Uuid::new_v4().to_string(),
            payload: Some(Payload::ConnectionListRequest(ConnectionListRequest {})),
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
            if let Some(Payload::ConnectionListResponse(list_resp)) = message.payload {
                for line in render_connection_entries(&list_resp.entries) {
                    println!("{line}");
                }
                return Ok(());
            }
        }

        bail!("did not receive connection list response")
    })
}

fn render_connection_entries(
    entries: &[slim_controller::api::proto::api::v1::ConnectionEntry],
) -> Vec<String> {
    entries
        .iter()
        .map(|entry| format!("id={} {}", entry.id, entry.config_data))
        .collect()
}

#[cfg(test)]
pub fn connection_list(_server: &str) -> Result<()> {
    let entries = vec![slim_controller::api::proto::api::v1::ConnectionEntry {
        id: 1,
        connection_type: 0,
        config_data: "config".to_string(),
    }];
    let _ = render_connection_entries(&entries);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::render_connection_entries;

    #[test]
    fn render_connection_entries_formats_lines() {
        let entries = vec![slim_controller::api::proto::api::v1::ConnectionEntry {
            id: 1,
            connection_type: 0,
            config_data: "config".to_string(),
        }];

        let lines = render_connection_entries(&entries);
        assert_eq!(lines, vec!["id=1 config".to_string()]);
    }
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
