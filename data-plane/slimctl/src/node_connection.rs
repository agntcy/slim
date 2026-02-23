use anyhow::{Context, Result, bail};
use slim_controller::api::proto::api::v1::control_message::Payload;
use slim_controller::api::proto::api::v1::controller_service_client::ControllerServiceClient;
use slim_controller::api::proto::api::v1::{ConnectionListRequest, ControlMessage};
use tokio::runtime::Builder;
use tokio_stream::iter;
use tonic::Request;
use tonic::transport::Channel;
use uuid::Uuid;

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
                for entry in list_resp.entries {
                    println!("id={} {}", entry.id, entry.config_data);
                }
                return Ok(());
            }
        }

        bail!("did not receive connection list response")
    })
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
