// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use tokio::sync::mpsc;
use tonic::Status;

use agp_gw_pubsub_proto::proto::pubsub::v1::Message;

#[derive(Debug, Clone, Default)]
pub(crate) enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
    #[default]
    Unkwon,
}

/// Connection type
#[derive(Debug, Clone, Default)]
pub(crate) enum Type {
    /// Connection with local application
    Local,

    /// Connection with remote gateway
    Remote,

    /// Unknown connection type
    #[default]
    Unkwon,
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

    /// Connection type
    connection_type: Type,
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

    /// Get the remote address
    pub(crate) fn remote_addr(&self) -> Option<&SocketAddr> {
        self.remote_addr.as_ref()
    }

    /// Get the local address
    pub(crate) fn local_addr(&self) -> Option<&SocketAddr> {
        self.local_addr.as_ref()
    }

    /// Get the channel
    pub(crate) fn channel(&self) -> &Channel {
        &self.channel
    }

    /// Get the connection type
    #[allow(dead_code)]
    pub(crate) fn connection_type(&self) -> &Type {
        &self.connection_type
    }
}
