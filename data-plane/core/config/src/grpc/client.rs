// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub use crate::client::*;

use tonic::codegen::{Body, Bytes, StdError};

use super::errors::ConfigError;

impl ClientConfig {
    /// Converts the client configuration to a gRPC-only channel.
    pub async fn to_grpc_channel(
        &self,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        >
        + Send
        + Clone
        + 'static
        + use<>,
        ConfigError,
    > {
        self.to_grpc_channel_internal().await
    }

    /// Internal gRPC channel builder used by both public gRPC API and transport switching.
    pub(crate) async fn to_grpc_channel_internal(
        &self,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        >
        + Send
        + Clone
        + 'static
        + use<>,
        ConfigError,
    > {
        self.to_channel_internal(false).await
    }

    /// Converts the client configuration to a gRPC-only channel without retry logic.
    #[cfg(test)]
    pub async fn to_grpc_channel_lazy(
        &self,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
        ConfigError,
    > {
        self.to_grpc_channel_lazy_internal().await
    }

    /// Internal lazy gRPC channel builder for tests.
    #[cfg(test)]
    pub(crate) async fn to_grpc_channel_lazy_internal(
        &self,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
        ConfigError,
    > {
        self.to_channel_internal(true).await
    }
}
