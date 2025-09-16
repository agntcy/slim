// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::api::proto::dataplane::v1::Message;
use slim_config::grpc::client::ClientConfig;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::Status;

#[derive(Debug, Clone, Default)]
pub(crate) enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
    #[default]
    Unknown,
}

/// Connection type
#[derive(Debug, Clone, Default)]
pub(crate) enum Type {
    /// Connection with local application
    Local,

    /// Connection with remote slim instance
    Remote,

    /// Unknown connection type
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
/// Connection information
pub struct Connection {
    /// Remote address and port. Not available for local connections
    remote_addr: Option<SocketAddr>,

    /// Local address and port. Not available for remote connections
    local_addr: Option<SocketAddr>,

    /// Channel to send messages
    channel: Channel,

    /// Configuration data for the connection.
    config_data: Option<ClientConfig>,

    /// Connection type
    connection_type: Type,

    /// cancellation token to stop the receiving loop on this connection
    cancellation_token: Option<CancellationToken>,
}

/// Implementation of Connection
impl Connection {
    /// Create a new Connection
    pub(crate) fn new(connection_type: Type) -> Self {
        Self {
            connection_type,
            ..Default::default()
        }
    }

    /// Set the remote address
    pub(crate) fn with_remote_addr(self, remote_addr: Option<SocketAddr>) -> Self {
        Self {
            remote_addr,
            ..self
        }
    }

    /// Set the local address
    pub(crate) fn with_local_addr(self, local_addr: Option<SocketAddr>) -> Self {
        Self { local_addr, ..self }
    }

    /// Set the channel to send messages
    pub(crate) fn with_channel(self, channel: Channel) -> Self {
        Self { channel, ..self }
    }

    /// Set the configuration data for the connection
    pub(crate) fn with_config_data(self, config_data: Option<ClientConfig>) -> Self {
        Self {
            config_data,
            ..self
        }
    }

    /// Get the remote address
    pub fn remote_addr(&self) -> Option<&SocketAddr> {
        self.remote_addr.as_ref()
    }

    /// Get the local address
    pub fn local_addr(&self) -> Option<&SocketAddr> {
        self.local_addr.as_ref()
    }

    /// Get the channel
    pub(crate) fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn config_data(&self) -> Option<&ClientConfig> {
        self.config_data.as_ref()
    }

    /// Get the connection type
    #[allow(dead_code)]
    pub(crate) fn connection_type(&self) -> &Type {
        &self.connection_type
    }

    /// Return true if is a local connection
    pub(crate) fn is_local_connection(&self) -> bool {
        matches!(self.connection_type, Type::Local)
    }

    /// Set cancellation token
    pub(crate) fn with_cancellation_token(
        self,
        cancellation_token: Option<CancellationToken>,
    ) -> Self {
        Self {
            cancellation_token,
            ..self
        }
    }

    /// Get cancellation token
    pub(crate) fn cancellation_token(&self) -> Option<&CancellationToken> {
        self.cancellation_token.as_ref()
    }
}
