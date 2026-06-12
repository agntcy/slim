// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use parking_lot::Mutex;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoName;

use crate::{
    Direction, SessionError, SlimChannelSender,
    common::{AppChannelSender, SessionMessage},
    session_config::SessionConfig,
    subscription_manager::{SubscriptionManager, SubscriptionOps},
};

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

    /// Seen control-message sequence numbers per remote sender (E2E replay protection).
    pub(crate) seen_control_seqs: Arc<Mutex<HashMap<ProtoName, HashSet<u64>>>>,
}

pub(crate) fn new_seen_control_seqs() -> Arc<Mutex<HashMap<ProtoName, HashSet<u64>>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

impl<P, V, M> SessionSettings<P, V, M>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    M: SubscriptionOps,
{
    /// Forget replay state for a participant that left or was removed so re-invites can restart seq.
    pub(crate) fn clear_seen_control_seqs(&self, participant: &ProtoName) {
        let components = participant.str_components();
        self.seen_control_seqs
            .lock()
            .retain(|k, _| k.str_components() != components);
    }
}
