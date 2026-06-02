// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Local crate
use crate::{common::SessionMessage, errors::SessionError, mls_state::MlsState};
use slim_auth::traits::{TokenProvider, Verifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingState {
    Active,
    Draining,
}

/// Core trait for message handling at any layer.
///
/// Each layer implements this trait and can hold an inner layer.
/// The layer decides whether to pass messages to its inner layer or handle them itself (or both).
#[trait_variant::make(Send)]
pub trait MessageHandler {
    /// Init the layer.
    async fn init(&mut self) -> Result<(), SessionError>;

    /// Process an incoming or outgoing message.
    ///
    /// # Arguments
    /// * `message` - The session message. It can be an actual message or an event.
    /// * `direction` - Whether the message is incoming (from network) or outgoing (from app)
    ///
    /// # Returns
    /// * `Ok(())` - If processing succeeded
    /// * `Err(SessionError)` - If processing failed
    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError>;

    /// Process a message with optional MLS state for outbound encryption at the transmitter.
    ///
    /// Layers that send to SLIM pass `mls` through to `SessionTransmitter::send_to_slim`.
    /// The default implementation ignores MLS and delegates to [`MessageHandler::on_message`].
    async fn on_message_with_mls<P, V>(
        &mut self,
        mls: Option<&mut MlsState<P, V>>,
        message: SessionMessage,
    ) -> Result<(), SessionError>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        let _ = mls;
        async { self.on_message(message).await }
    }

    /// Add an endpoint to the session.
    /// Default implementation does nothing for layers that don't manage endpoints.
    async fn add_endpoint(
        &mut self,
        _endpoint: &slim_datapath::api::Participant,
    ) -> Result<(), SessionError> {
        async { Ok(()) }
    }

    /// Add an endpoint, passing MLS state for outbound encryption on flush.
    async fn add_endpoint_with_mls<P, V>(
        &mut self,
        _mls: Option<&mut MlsState<P, V>>,
        endpoint: &slim_datapath::api::Participant,
    ) -> Result<(), SessionError>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        async { self.add_endpoint(endpoint).await }
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
}
