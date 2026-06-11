// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::api::proto::dataplane::v1::Message;
use semver::Version;
use slim_config::client::ClientConfig;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::header_mac::HeaderMacSession;

#[derive(Debug, Clone)]
pub enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
}

use crate::tables::ConnType;

#[derive(Clone)]
/// Connection information.
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
    connection_type: ConnType,

    /// cancellation token to stop the receiving loop on this connection
    cancellation_token: Option<CancellationToken>,

    /// Link identifier shared between both sides of a remote link.
    link_id: Option<String>,

    /// SLIM version of the remote peer (set during negotiation).
    remote_slim_version: Option<Version>,

    /// HMAC session derived from the ECDH key exchange (set during negotiation).
    header_hmac: Option<HeaderMacSession>,

    /// Strict header MAC policy for this connection (fixed at establishment).
    require_header_mac: bool,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("remote_addr", &self.remote_addr)
            .field("local_addr", &self.local_addr)
            .field("channel", &self.channel)
            .field("config_data", &self.config_data)
            .field("connection_type", &self.connection_type)
            .field("link_id", &self.link_id)
            .field("remote_slim_version", &self.remote_slim_version)
            .field("header_hmac", &self.header_hmac.is_some())
            .finish_non_exhaustive()
    }
}

/// Implementation of Connection
impl Connection {
    /// Create a new Connection
    pub fn new(connection_type: ConnType, channel: Channel) -> Self {
        Self {
            remote_addr: None,
            local_addr: None,
            channel,
            config_data: None,
            connection_type,
            cancellation_token: None,
            link_id: None,
            remote_slim_version: None,
            header_hmac: None,
            require_header_mac: false,
        }
    }

    /// Set whether strict header MAC verification applies on this connection.
    pub(crate) fn with_require_header_mac(self, require_header_mac: bool) -> Self {
        Self {
            require_header_mac,
            ..self
        }
    }

    pub(crate) fn require_header_mac(&self) -> bool {
        self.require_header_mac
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

    pub(crate) fn header_hmac(&self) -> Option<&HeaderMacSession> {
        self.header_hmac.as_ref()
    }

    pub(crate) fn install_header_hmac(&mut self, mac: HeaderMacSession) {
        self.header_hmac = Some(mac);
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

    /// Return true if is a local connection
    pub(crate) fn is_local_connection(&self) -> bool {
        matches!(self.connection_type, ConnType::Local)
    }

    /// Return true if is a peer connection (same deployment replica)
    #[allow(dead_code)]
    pub(crate) fn is_peer_connection(&self) -> bool {
        matches!(self.connection_type, ConnType::Peer)
    }

    /// Return the connection category for subscription table operations.
    pub(crate) fn category(&self) -> ConnType {
        self.connection_type
    }

    /// Return true if this node initiated the connection (outbound dial).
    ///
    /// gRPC inbound peers use [`Channel::Server`]; outbound dials use [`Channel::Client`]
    /// with [`config_data`](Self::config_data) set from [`ClientConfig`].
    ///
    /// WebSocket is asymmetric: the server accept path still uses [`Channel::Client`] for
    /// writes, but leaves `config_data` unset, so inbound WebSocket is distinguished from
    /// outbound WebSocket (which always carries `config_data` from the dial).
    pub fn is_outgoing(&self) -> bool {
        matches!(self.channel, Channel::Client(_)) && self.config_data.is_some()
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

    /// Set the link identifier at construction time (client side).
    pub(crate) fn with_link_id(mut self, link_id: String) -> Self {
        self.link_id = Some(link_id);
        self
    }

    /// Set the shared link identifier for this connection.
    pub fn set_link_id(&mut self, link_id: String) {
        self.link_id = Some(link_id);
    }

    /// Get the shared link identifier for this connection.
    pub fn link_id(&self) -> Option<String> {
        self.link_id.clone()
    }

    /// Get the SLIM version of the remote peer.
    pub fn remote_slim_version(&self) -> Option<Version> {
        self.remote_slim_version.clone()
    }

    /// Returns true if link negotiation has completed (remote_slim_version is set).
    pub fn is_negotiated(&self) -> bool {
        self.remote_slim_version.is_some()
    }

    /// Complete link negotiation on the server (incoming) path.
    ///
    /// Stores `link_id` and `version`. Returns `false` if `link_id` is empty or
    /// negotiation is already complete (replay protection).
    pub fn complete_negotiation_as_server(&mut self, link_id: &str, version: Version) -> bool {
        if self.remote_slim_version.is_some() {
            return false;
        }
        if link_id.is_empty() {
            return false;
        }
        self.link_id = Some(link_id.to_string());
        self.remote_slim_version = Some(version);
        true
    }

    /// Complete link negotiation on the client (outgoing) path.
    ///
    /// Verifies the echoed `link_id` matches what was set, then stores `version`.
    /// Returns `false` if there is a mismatch or negotiation is already complete.
    pub fn complete_negotiation_as_client(&mut self, link_id: &str, version: Version) -> bool {
        if self.remote_slim_version.is_some() {
            return false;
        }
        if self.link_id.as_deref() != Some(link_id) {
            return false;
        }
        self.remote_slim_version = Some(version);
        true
    }

    /// Send a message directly through this connection's channel.
    pub(crate) async fn send(&self, msg: Message) -> Result<(), crate::errors::DataPathError> {
        match &self.channel {
            Channel::Server(tx) => tx
                .send(Ok(msg))
                .await
                .map_err(|_| crate::errors::DataPathError::ConnectionSendError),
            Channel::Client(tx) => tx
                .send(msg)
                .await
                .map_err(|_| crate::errors::DataPathError::ConnectionSendError),
        }
    }

    /// Set negotiation state at construction time.
    pub fn with_negotiation(mut self, link_id: &str, version: &str) -> Self {
        self.link_id = Some(link_id.to_string());
        self.remote_slim_version = version.parse().ok();
        self
    }

    /// Set header HMAC at construction time (for testing).
    #[cfg(test)]
    pub(crate) fn with_header_hmac(mut self, mac: HeaderMacSession) -> Self {
        self.header_hmac = Some(mac);
        self
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs};

    use super::*;
    use tokio::sync::mpsc;

    fn server_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(ConnType::Remote, Channel::Server(tx))
    }

    fn client_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(ConnType::Remote, Channel::Client(tx))
            .with_config_data(Some(ClientConfig::default()))
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
    fn test_is_outgoing_websocket_inbound() {
        let (tx, _rx) = mpsc::channel(1);
        let conn = Connection::new(ConnType::Remote, Channel::Client(tx));
        assert!(!conn.is_outgoing());
    }

    #[test]
    fn test_link_id_initially_none() {
        assert!(server_conn().link_id().is_none());
    }

    #[test]
    fn test_connection_format_print() {
        let remote = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080)
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();

        let local = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8081)
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();

        let conn = client_conn()
            .with_remote_addr(Some(remote))
            .with_local_addr(Some(local));
        let debug = format!("{conn:?}");

        assert!(debug.starts_with("Connection"));
        assert!(debug.contains("connection_type: Remote"));
        assert!(debug.contains("remote_addr: Some"));
        assert!(debug.contains("local_addr: Some"));
    }

    #[test]
    fn test_set_and_get_link_id() {
        let mut conn = server_conn();
        conn.set_link_id("my-link".to_string());
        assert_eq!(conn.link_id(), Some("my-link".to_string()));
    }

    #[test]
    fn test_remote_slim_version_initially_none() {
        assert!(server_conn().remote_slim_version().is_none());
    }

    #[test]
    fn test_is_negotiated_initially_false() {
        assert!(!server_conn().is_negotiated());
        assert!(!client_conn().is_negotiated());
    }

    #[test]
    fn test_is_negotiated_true_after_server_negotiation() {
        let mut conn = server_conn();
        conn.complete_negotiation_as_server("link-id", Version::parse("1.0.0").unwrap());
        assert!(conn.is_negotiated());
    }

    #[test]
    fn test_is_negotiated_true_after_client_negotiation() {
        let mut conn = client_conn();
        let id = uuid::Uuid::new_v4().to_string();
        conn.set_link_id(id.clone());
        conn.complete_negotiation_as_client(&id, Version::parse("1.0.0").unwrap());
        assert!(conn.is_negotiated());
    }

    #[test]
    fn test_complete_negotiation_as_server_stores_link_id() {
        let mut conn = server_conn();
        let id = "my-custom-link-id";
        let v = Version::parse("1.2.3").unwrap();
        assert!(conn.complete_negotiation_as_server(id, v.clone()));
        assert_eq!(conn.link_id(), Some(id.to_string()));
        assert_eq!(conn.remote_slim_version(), Some(v));
    }

    #[test]
    fn test_complete_negotiation_as_server_rejects_empty_link_id() {
        let mut conn = server_conn();
        assert!(!conn.complete_negotiation_as_server("", Version::parse("1.0.0").unwrap()));
        assert!(conn.link_id().is_none());
        assert!(conn.remote_slim_version().is_none());
    }

    #[test]
    fn test_complete_negotiation_as_server_replay_returns_false() {
        let mut conn = server_conn();
        let id = uuid::Uuid::new_v4().to_string();
        let v1 = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_server(&id, v1.clone()));
        // Second call must be rejected; state must not change.
        assert!(!conn.complete_negotiation_as_server(&id, Version::parse("2.0.0").unwrap()));
        assert_eq!(conn.remote_slim_version(), Some(v1));
    }

    #[test]
    fn test_complete_negotiation_as_client_accepts_matching_link_id() {
        let mut conn = client_conn();
        let id = uuid::Uuid::new_v4().to_string();
        conn.set_link_id(id.clone());
        let v = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_client(&id, v.clone()));
        assert_eq!(conn.remote_slim_version(), Some(v));
    }

    #[test]
    fn test_complete_negotiation_as_client_rejects_mismatched_link_id() {
        let mut conn = client_conn();
        conn.set_link_id(uuid::Uuid::new_v4().to_string());
        assert!(!conn.complete_negotiation_as_client("wrong-id", Version::parse("1.0.0").unwrap()));
        assert!(conn.remote_slim_version().is_none());
    }

    #[test]
    fn test_complete_negotiation_as_client_replay_returns_false() {
        let mut conn = client_conn();
        let id = uuid::Uuid::new_v4().to_string();
        conn.set_link_id(id.clone());
        let v1 = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_client(&id, v1.clone()));
        // Second call must be rejected; state must not change.
        assert!(!conn.complete_negotiation_as_client(&id, Version::parse("2.0.0").unwrap()));
        assert_eq!(conn.remote_slim_version(), Some(v1));
    }
}
