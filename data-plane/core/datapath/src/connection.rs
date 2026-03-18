// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::api::proto::dataplane::v1::Message;
use parking_lot::RwLock;
use semver::Version;
use slim_config::grpc::client::ClientConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::Status;

/// Negotiation state shared between link negotiation fields.
/// Kept under one lock so that the check-and-set is atomic.
#[derive(Debug, Default)]
struct NegotiationState {
    link_id: Option<String>,
    remote_slim_version: Option<Version>,
}

#[derive(Debug, Clone)]
pub(crate) enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
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

#[derive(Debug, Clone)]
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

    /// Link negotiation state (link_id + remote_slim_version) under one lock for atomic check-and-set.
    negotiation: Arc<RwLock<NegotiationState>>,
}

/// Implementation of Connection
impl Connection {
    /// Create a new Connection
    pub(crate) fn new(connection_type: Type, channel: Channel) -> Self {
        Self {
            remote_addr: None,
            local_addr: None,
            channel,
            config_data: None,
            connection_type,
            cancellation_token: None,
            negotiation: Arc::new(RwLock::new(NegotiationState::default())),
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

    /// Return true if this node initiated the connection (client side).
    /// False means the remote peer connected to us (server side).
    pub(crate) fn is_outgoing(&self) -> bool {
        matches!(self.channel, Channel::Client(_))
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

    /// Set the shared link identifier for this connection.
    /// Used by the client before sending the initial negotiation request.
    pub fn set_link_id(&self, link_id: String) {
        self.negotiation.write().link_id = Some(link_id);
    }

    /// Get the shared link identifier for this connection.
    pub fn link_id(&self) -> Option<String> {
        self.negotiation.read().link_id.clone()
    }

    /// Get the SLIM version of the remote peer.
    pub fn remote_slim_version(&self) -> Option<Version> {
        self.negotiation.read().remote_slim_version.clone()
    }

    /// Atomically complete link negotiation.
    ///
    /// If negotiation is already complete (`remote_slim_version` is `Some`), returns `false`
    /// without modifying any state (replay protection).
    ///
    /// Otherwise, optionally stores `link_id` and stores `version`, then returns `true`.
    /// The check and set happen under a single write lock, eliminating TOCTOU races.
    pub fn complete_negotiation(&self, link_id: Option<String>, version: Version) -> bool {
        let mut state = self.negotiation.write();
        if state.remote_slim_version.is_some() {
            return false;
        }
        if let Some(id) = link_id {
            state.link_id = Some(id);
        }
        state.remote_slim_version = Some(version);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn server_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(Type::Remote, Channel::Server(tx))
    }

    fn client_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(Type::Remote, Channel::Client(tx))
    }

    #[test]
    fn test_is_outgoing_client() {
        assert!(client_conn().is_outgoing());
    }

    #[test]
    fn test_is_outgoing_server() {
        assert!(!server_conn().is_outgoing());
    }

    #[test]
    fn test_link_id_initially_none() {
        assert!(server_conn().link_id().is_none());
    }

    #[test]
    fn test_set_and_get_link_id() {
        let conn = server_conn();
        conn.set_link_id("my-link".to_string());
        assert_eq!(conn.link_id(), Some("my-link".to_string()));
    }

    #[test]
    fn test_remote_slim_version_initially_none() {
        assert!(server_conn().remote_slim_version().is_none());
    }

    #[test]
    fn test_complete_negotiation_first_call_stores_state() {
        let conn = server_conn();
        let v = Version::parse("1.2.3").unwrap();
        assert!(conn.complete_negotiation(Some("id".to_string()), v.clone()));
        assert_eq!(conn.link_id(), Some("id".to_string()));
        assert_eq!(conn.remote_slim_version(), Some(v));
    }

    #[test]
    fn test_complete_negotiation_replay_returns_false() {
        let conn = server_conn();
        let v1 = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation(None, v1.clone()));
        // Second call must be rejected; state must not change.
        assert!(!conn.complete_negotiation(None, Version::parse("2.0.0").unwrap()));
        assert_eq!(conn.remote_slim_version(), Some(v1));
    }

    #[test]
    fn test_complete_negotiation_none_link_id_preserves_existing() {
        let conn = server_conn();
        conn.set_link_id("original".to_string());
        assert!(conn.complete_negotiation(None, Version::parse("1.0.0").unwrap()));
        assert_eq!(conn.link_id(), Some("original".to_string()));
    }
}
