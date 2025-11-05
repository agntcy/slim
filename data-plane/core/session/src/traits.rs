// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use async_trait::async_trait;
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use super::SessionInterceptorProvider;
use crate::{common::SessionMessage, errors::SessionError};

/// Session transmitter trait
pub trait Transmitter: SessionInterceptorProvider {
    fn send_to_slim(
        &self,
        message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static;

    fn send_to_app(
        &self,
        message: Result<Message, SessionError>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static;
}

/// Core trait for message handling at any layer.
///
/// Each layer implements this trait and can hold an inner layer.
/// The layer decides whether to pass messages to its inner layer or handle them itself (or both).
#[async_trait]
pub trait MessageHandler: Send + Sync {
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

    /// Optional hook called before the layer is shut down.
    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    /// Optional hook for periodic ops (e.g. MLS key rotation)
    #[allow(dead_code)]
    async fn on_tick(&mut self) -> Result<(), SessionError> {
        Ok(())
    }
}
