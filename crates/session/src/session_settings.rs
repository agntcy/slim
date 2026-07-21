// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::num::NonZeroUsize;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoName;

use crate::{
    Direction, SessionError, SlimChannelSender,
    common::{AppChannelSender, SessionMessage},
    session_config::SessionConfig,
    subscription_manager::{SubscriptionManager, SubscriptionOps},
};

/// Max size of control message replay cache
pub(crate) const DEFAULT_MAX_SEEN_CONTROL_MESSAGE_IDS_SIZE: NonZeroUsize =
    NonZeroUsize::new(1000).unwrap();
pub(crate) const MAX_SEEN_CONTROL_MESSAGE_SENDERS_SIZE: usize = 100;

/// Settings struct for constructing session components.
///
/// This struct encapsulates all the parameters needed to construct
/// `SessionParticipant`, `SessionModerator`, and `SessionController`.
/// It reduces the number of parameters passed to internal constructors
/// and provides a clean internal API.
///
/// # Note
///
/// This struct is primarily for internal use. External users should use
/// the `SessionBuilder` for a more ergonomic API.
#[derive(Clone)]
pub(crate) struct SessionSettings<P, V, M = SubscriptionManager>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    M: SubscriptionOps,
{
    /// Session ID
    pub(crate) id: u32,

    /// Local endpoint name
    pub(crate) source: ProtoName,

    /// Remote endpoint or group name
    /// used to send application data
    pub(crate) destination: ProtoName,

    /// Group name for control messages
    /// in P2P session is the same as destination
    pub(crate) control: ProtoName,

    /// Session configuration
    pub(crate) config: SessionConfig,

    /// Direction for data message flow (send, receive, both, or none)
    pub(crate) direction: Direction,

    /// Channel for sending messages to SLIM
    pub(crate) slim_tx: SlimChannelSender,

    /// Channel for sending messages to the application
    pub(crate) app_tx: AppChannelSender,

    /// Tx channel for sending messages to session queue
    pub(crate) tx_session: tokio::sync::mpsc::Sender<SessionMessage>,

    /// Channel to send messages to the session layer
    pub(crate) tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// Identity token provider
    pub(crate) identity_provider: P,

    /// Identity token verifier
    pub(crate) identity_verifier: V,

    /// Graceful shutdown timeout - time to drain pending messages during shutdown
    pub(crate) graceful_shutdown_timeout: Option<std::time::Duration>,

    /// Subscription manager for ACK-aware subscribe/unsubscribe operations
    pub(crate) subscription_manager: M,

    /// Service ID for tracing — identifies which service instance owns this session
    pub(crate) service_id: String,

    /// Seen control messages cache max size (for replay attack prevention)
    pub(crate) max_seen_control_message_ids_size: NonZeroUsize,

    /// Node TLS policy: when true, new MLS groups use hybrid PQ KEM (native only).
    pub(crate) enforce_pqc: bool,
}
