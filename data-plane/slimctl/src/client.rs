// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use tonic::codegen::{Body, Bytes, StdError};

use crate::proto::channel_manager::proto::v1::channel_manager_service_client::ChannelManagerServiceClient;
use crate::proto::controller::proto::v1::controller_service_client::ControllerServiceClient;
use crate::proto::controlplane::proto::v1::control_plane_service_client::ControlPlaneServiceClient;
use slim_config::grpc::client::ClientConfig;

/// Create a `ControlPlaneServiceClient` with TLS and auth configured from `opts`.
pub async fn get_control_plane_client(
    opts: &ClientConfig,
) -> Result<
    ControlPlaneServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + Send> + Send + 'static,
            Future: Send,
        > + Clone
        + Send
        + 'static,
    >,
> {
    let channel = opts
        .to_channel()
        .await
        .context("failed to connect to server")?;
    Ok(ControlPlaneServiceClient::new(channel))
}

/// Create a `ControllerServiceClient` with TLS and auth configured from `opts`.
pub async fn get_controller_client(
    opts: &ClientConfig,
) -> Result<
    ControllerServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + Send> + Send + 'static,
            Future: Send,
        > + Clone
        + Send
        + 'static,
    >,
> {
    let channel = opts
        .to_channel()
        .await
        .context("failed to connect to server")?;
    Ok(ControllerServiceClient::new(channel))
}

/// Create a `ChannelManagerServiceClient` with TLS and auth configured from `opts`.
pub async fn get_channel_manager_client(
    opts: &ClientConfig,
) -> Result<
    ChannelManagerServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + Send> + Send + 'static,
            Future: Send,
        > + Clone
        + Send
        + 'static,
    >,
> {
    let channel = opts
        .to_channel()
        .await
        .context("failed to connect to channel manager")?;
    Ok(ChannelManagerServiceClient::new(channel))
}

/// Call a unary RPC method, wrapping the request in `tonic::Request::new()`
/// and unwrapping the inner response value.
/// The channel-level `request_timeout` (set via `ClientConfig`) applies automatically.
///
/// Usage: `rpc!(client, method_name, request)`
#[macro_export]
macro_rules! rpc {
    ($client:expr, $method:ident, $req:expr) => {
        $client
            .$method(tonic::Request::new($req))
            .await?
            .into_inner()
    };
}
