// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use async_trait::async_trait;
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use super::SessionInterceptorProvider;
use crate::{common::SessionMessage, errors::SessionError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingState {
    Active,
    Draining,
}

/// Session transmitter trait
#[async_trait]
pub trait Transmitter: SessionInterceptorProvider {
    async fn send_to_slim(&self, message: Result<Message, Status>) -> Result<(), SessionError>;

    async fn send_to_app(&self, message: Result<Message, SessionError>)
    -> Result<(), SessionError>;
}

/// Core trait for message handling at any layer.
///
/// Each layer implements this trait and can hold an inner layer.
/// The layer decides whether to pass messages to its inner layer or handle them itself (or both).
#[async_trait]
pub trait MessageHandler: Send + Sync {
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

    /// Add an endpoint to the session.
    /// Default implementation does nothing for layers that don't manage endpoints.
    async fn add_endpoint(
        &mut self,
        _endpoint: &slim_datapath::messages::Name,
    ) -> Result<(), SessionError> {
        Ok(())
    }

    /// Remove an endpoint from the session.
    /// Default implementation does nothing for layers that don't manage endpoints.
    fn remove_endpoint(&mut self, _endpoint: &slim_datapath::messages::Name) {
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
        Ok(())
    }
}
