// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::api::proto::dataplane::v1::Message;
use aws_lc_rs::agreement::EphemeralPrivateKey;
use parking_lot::RwLock;
use semver::Version;
use slim_config::client::{ClientConfig, is_valid_uuid_v4};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::header_mac::HeaderMacSession;

/// Negotiation state shared between link negotiation fields.
/// Kept under one lock so that the check-and-set is atomic.
#[derive(Debug, Default)]
struct NegotiationState {
    link_id: Option<String>,
    remote_slim_version: Option<Version>,
    pub(crate) header_hmac: Option<Arc<HeaderMacSession>>,
    outbound_ecdh_private: Option<EphemeralPrivateKey>,
}

#[derive(Debug, Clone)]
pub(crate) enum Channel {
    Server(mpsc::Sender<Result<Message, Status>>),
    Client(mpsc::Sender<Message>),
}

use crate::tables::ConnCategory;

#[derive(Clone)]
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
    connection_type: ConnCategory,

    /// cancellation token to stop the receiving loop on this connection
    cancellation_token: Option<CancellationToken>,

    /// Link negotiation state (link_id + remote_slim_version) under one lock for atomic check-and-set.
    negotiation: Arc<RwLock<NegotiationState>>,

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
            // Not printing sensitive data
            .field("negotiation", &"NegotiationState")
            .finish_non_exhaustive()
    }
}

/// Implementation of Connection
impl Connection {
    /// Create a new Connection
    pub(crate) fn new(connection_type: ConnCategory, channel: Channel) -> Self {
        Self {
            remote_addr: None,
            local_addr: None,
            channel,
            config_data: None,
            connection_type,
            cancellation_token: None,
            negotiation: Arc::new(RwLock::new(NegotiationState::default())),
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

    pub(crate) fn header_hmac(&self) -> Option<Arc<HeaderMacSession>> {
        self.negotiation.read().header_hmac.clone()
    }

    pub(crate) fn take_outbound_ecdh_private(&self) -> Option<EphemeralPrivateKey> {
        self.negotiation.write().outbound_ecdh_private.take()
    }

    pub(crate) fn set_outbound_ecdh_private(&self, key: EphemeralPrivateKey) {
        self.negotiation.write().outbound_ecdh_private = Some(key);
    }

    pub(crate) fn install_header_hmac(&self, mac: Arc<HeaderMacSession>) {
        self.negotiation.write().header_hmac = Some(mac);
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
    pub(crate) fn connection_type(&self) -> &ConnCategory {
        &self.connection_type
    }

    /// Return true if is a local connection
    pub(crate) fn is_local_connection(&self) -> bool {
        matches!(self.connection_type, ConnCategory::Local)
    }

    /// Return true if is a peer connection (same deployment replica)
    #[allow(dead_code)]
    pub(crate) fn is_peer_connection(&self) -> bool {
        matches!(self.connection_type, ConnCategory::Peer)
    }

    /// Return the connection category for subscription table operations.
    pub(crate) fn conn_category(&self) -> ConnCategory {
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

    /// Set the link identifier at construction time so it is available the moment the
    /// connection enters the table, before the negotiation message is sent.
    pub(crate) fn with_link_id(self, link_id: String) -> Self {
        self.negotiation.write().link_id = Some(link_id);
        self
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

    /// Atomically complete link negotiation on the server (incoming) path.
    ///
    /// Validates `link_id` as a UUID v4 and stores it together with `version` under one lock.
    /// Returns `false` if `link_id` is not a valid UUID v4 or negotiation is already complete
    /// (replay protection).
    pub fn complete_negotiation_as_server(&self, link_id: &str, version: Version) -> bool {
        let mut state = self.negotiation.write();
        if state.remote_slim_version.is_some() {
            return false;
        }
        if !is_valid_uuid_v4(link_id) {
            return false;
        }
        state.link_id = Some(link_id.to_string());
        state.remote_slim_version = Some(version);
        true
    }

    /// Atomically complete link negotiation on the client (outgoing) path.
    ///
    /// Verifies the echoed `link_id` matches what was stored by `set_link_id`, then stores
    /// `version`, all under one lock.  Returns `false` if there is a mismatch or negotiation
    /// is already complete (replay protection).
    pub fn complete_negotiation_as_client(&self, link_id: &str, version: Version) -> bool {
        let mut state = self.negotiation.write();
        if state.remote_slim_version.is_some() {
            return false;
        }
        if state.link_id.as_deref() != Some(link_id) {
            return false;
        }
        state.remote_slim_version = Some(version);
        true
    }

    #[cfg(test)]
    pub(crate) fn test_install_header_mac(&self, mac: Arc<HeaderMacSession>) {
        self.install_header_hmac(mac);
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs};

    use super::*;
    use tokio::sync::mpsc;

    fn server_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(ConnCategory::Remote, Channel::Server(tx))
    }

    fn client_conn() -> Connection {
        let (tx, _rx) = mpsc::channel(1);
        Connection::new(ConnCategory::Remote, Channel::Client(tx))
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
        let conn = Connection::new(ConnCategory::Remote, Channel::Client(tx));
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
        // Sensitive fields are reducted
        assert!(debug.contains(r#"negotiation: "NegotiationState""#));
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
    fn test_complete_negotiation_as_server_stores_valid_uuid() {
        let conn = server_conn();
        let id = uuid::Uuid::new_v4().to_string();
        let v = Version::parse("1.2.3").unwrap();
        assert!(conn.complete_negotiation_as_server(&id, v.clone()));
        assert_eq!(conn.link_id(), Some(id));
        assert_eq!(conn.remote_slim_version(), Some(v));
    }

    #[test]
    fn test_complete_negotiation_as_server_rejects_invalid_uuid() {
        let conn = server_conn();
        assert!(
            !conn.complete_negotiation_as_server("not-a-uuid", Version::parse("1.0.0").unwrap())
        );
        assert!(conn.link_id().is_none());
        assert!(conn.remote_slim_version().is_none());
    }

    #[test]
    fn test_complete_negotiation_as_server_replay_returns_false() {
        let conn = server_conn();
        let id = uuid::Uuid::new_v4().to_string();
        let v1 = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_server(&id, v1.clone()));
        // Second call must be rejected; state must not change.
        assert!(!conn.complete_negotiation_as_server(&id, Version::parse("2.0.0").unwrap()));
        assert_eq!(conn.remote_slim_version(), Some(v1));
    }

    #[test]
    fn test_complete_negotiation_as_client_accepts_matching_link_id() {
        let conn = client_conn();
        let id = uuid::Uuid::new_v4().to_string();
        conn.set_link_id(id.clone());
        let v = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_client(&id, v.clone()));
        assert_eq!(conn.remote_slim_version(), Some(v));
    }

    #[test]
    fn test_complete_negotiation_as_client_rejects_mismatched_link_id() {
        let conn = client_conn();
        conn.set_link_id(uuid::Uuid::new_v4().to_string());
        assert!(!conn.complete_negotiation_as_client("wrong-id", Version::parse("1.0.0").unwrap()));
        assert!(conn.remote_slim_version().is_none());
    }

    #[test]
    fn test_complete_negotiation_as_client_replay_returns_false() {
        let conn = client_conn();
        let id = uuid::Uuid::new_v4().to_string();
        conn.set_link_id(id.clone());
        let v1 = Version::parse("1.0.0").unwrap();
        assert!(conn.complete_negotiation_as_client(&id, v1.clone()));
        // Second call must be rejected; state must not change.
        assert!(!conn.complete_negotiation_as_client(&id, Version::parse("2.0.0").unwrap()));
        assert_eq!(conn.remote_slim_version(), Some(v1));
    }
}
