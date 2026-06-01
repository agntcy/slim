// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Peer discovery abstraction for SLIM replicas.
//!
//! Provides a trait-based abstraction for discovering peer replicas within the
//! same deployment. Implementations include:
//! - [`StaticPeerDiscovery`]: Configuration-defined list of peer endpoints
//! - (Future) Kubernetes API-based discovery

pub mod config;
mod static_list;

pub use config::{PeerConfig, PeerDiscoveryConfig, PeerTopology, StaticPeerEntry};
pub use static_list::StaticPeerDiscovery;

use std::fmt;

use slim_config::client::ClientConfig;

/// Information about a discovered peer replica.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Unique peer identifier (e.g., pod name or configured ID).
    pub id: String,
    /// Full client configuration for connecting to the peer.
    pub config: ClientConfig,
}

/// Events emitted by a peer discovery implementation.
#[derive(Debug, Clone)]
pub enum PeerEvent {
    /// A new peer has been discovered and is available for connection.
    Joined(PeerInfo),
    /// A previously discovered peer is no longer available.
    Left(PeerInfo),
}

/// Errors that can occur during peer discovery.
#[derive(Debug)]
pub enum PeerDiscoveryError {
    /// Discovery backend failed to start or encountered an unrecoverable error.
    Backend(String),
    /// The discovery stream has been closed (clean shutdown).
    Closed,
}

impl fmt::Display for PeerDiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(msg) => write!(f, "peer discovery error: {msg}"),
            Self::Closed => write!(f, "peer discovery stream closed"),
        }
    }
}

impl std::error::Error for PeerDiscoveryError {}

/// Trait for peer discovery backends.
///
/// Implementations are responsible for discovering peer replicas and emitting
/// [`PeerEvent`]s as peers join or leave the deployment.
///
/// # Usage
///
/// ```ignore
/// let mut discovery = StaticPeerDiscovery::new(config, "self-id");
/// discovery.start().await?;
/// while let Ok(event) = discovery.recv().await {
///     match event {
///         PeerEvent::Joined(info) => { /* connect to peer */ }
///         PeerEvent::Left(info) => { /* disconnect from peer */ }
///     }
/// }
/// ```
#[trait_variant::make(Send)]
pub trait PeerDiscovery {
    /// Start the discovery process.
    ///
    /// This should return quickly. For static discovery, it emits all configured
    /// peers. For dynamic backends (e.g., Kubernetes), it begins watching for changes.
    async fn start(&mut self) -> Result<(), PeerDiscoveryError>;

    /// Receive the next peer event.
    ///
    /// Blocks until an event is available. Returns an error if the discovery
    /// backend encounters a failure or the stream is closed.
    async fn recv(&mut self) -> Result<PeerEvent, PeerDiscoveryError>;
}
