// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Local crate
use crate::{
    common::{SessionMessage, SessionOutput},
    errors::SessionError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingState {
    Active,
    Draining,
}

/// Core trait for message handling at any layer.
///
/// Each layer implements this trait and can hold an inner layer.
/// The layer decides whether to pass messages to its inner layer or handle them itself (or both).
///
/// Layers return `SessionOutput` containing outbound messages instead of sending them internally.
/// The processing loop is the single orchestration point for identity-setting and channel sends.
// On native the layer futures must be `Send` (multi-threaded tokio runtime).
// On wasm32 the MLS crypto provider (SubtleCrypto) yields `!Send` futures and
// the runtime is single-threaded, so the `Send` bound is dropped there.
#[cfg_attr(not(target_arch = "wasm32"), trait_variant::make(Send))]
#[cfg_attr(target_arch = "wasm32", trait_variant::make())]
pub trait MessageHandler {
    /// Init the layer.
    async fn init(&mut self) -> Result<(), SessionError>;

    /// Process an incoming or outgoing message.
    ///
    /// # Arguments
    /// * `message` - The session message. It can be an actual message or an event.
    ///
    /// # Returns
    /// * `Ok(SessionOutput)` - Outbound messages to send (may be empty)
    /// * `Err(SessionError)` - If processing failed
    async fn on_message(&mut self, message: SessionMessage) -> Result<SessionOutput, SessionError>;

    /// Add an endpoint to the session.
    /// Default implementation does nothing for layers that don't manage endpoints.
    async fn add_endpoint(
        &mut self,
        _endpoint: &slim_datapath::api::Participant,
    ) -> Result<SessionOutput, SessionError> {
        async { Ok(SessionOutput::new()) }
    }

    /// Remove an endpoint from the session.
    /// Default implementation does nothing for layers that don't manage endpoints.
    fn remove_endpoint(&mut self, _endpoint: &slim_datapath::api::ProtoName) {
        // Default: do nothing
    }

    /// Indicates whether the layer needs to drain messages before shutdown.
    fn needs_drain(&self) -> bool;

    /// Returns the current processing state (Active or Draining).
    /// Default implementation returns Active.
    fn processing_state(&self) -> ProcessingState {
        ProcessingState::Active
    }

    /// Optional hook called before the layer is shut down.
    async fn on_shutdown(&mut self) -> Result<(), SessionError>;

    /// Optional hook for periodic ops (e.g. MLS key rotation)
    #[allow(dead_code)]
    async fn on_tick(&mut self) -> Result<(), SessionError> {
        async { Ok(()) }
    }

    /// Get the current participants list (participants in the session)
    /// Default implementation returns an empty list.
    fn participants_list(&self) -> Vec<slim_datapath::api::ProtoName> {
        vec![]
    }

    /// Returns the names of participants that have not yet acked the given message id.
    /// Default implementation returns an empty list.
    fn missing_acks_for(&self, _id: u32) -> Vec<slim_datapath::api::ProtoName> {
        vec![]
    }
}

pub trait MlsStateSelector<P, V>: Send + Sync
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    fn set_mls_state(
        &mut self,
        mls_state: std::sync::Arc<parking_lot::Mutex<crate::mls_state::MlsState<P, V>>>,
    );
}
