// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;

use crate::{
    common::SessionMessage, errors::SessionError, session_config::SessionConfig,
    transmitter::SessionTransmitter,
};

/// Common fields shared between session components.
///
/// This struct contains the core session identification and communication
/// channels that are used by both the construction settings and runtime state.
#[derive(Clone)]
pub(crate) struct SessionCommonFields {
    /// Session ID
    pub(crate) id: u32,

    /// Local endpoint name
    pub(crate) source: Name,

    /// Remote endpoint or group name
    pub(crate) destination: Name,

    /// Session configuration
    pub(crate) config: SessionConfig,

    /// Transmitter for sending messages
    pub(crate) tx: SessionTransmitter,

    /// Channel to communicate with session layer
    pub(crate) tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
}

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
pub(crate) struct SessionSettings<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Common session fields
    pub(crate) common: SessionCommonFields,

    /// Identity token provider
    pub(crate) identity_provider: P,

    /// Identity token verifier
    pub(crate) identity_verifier: V,

    /// Storage path for session data
    pub(crate) storage_path: std::path::PathBuf,
}
