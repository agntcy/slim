// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use async_trait::async_trait;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::Name;

// Local crate
use crate::session::{Id, MessageDirection, common::State};
use super::SessionConfig;
use super::SessionError;
use super::SessionInterceptorProvider;

pub trait SessionConfigTrait {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError>;
}

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

#[async_trait]
pub(crate) trait CommonSession<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session ID
    #[allow(dead_code)]
    fn id(&self) -> Id;

    // get the session state
    #[allow(dead_code)]
    fn state(&self) -> &State;

    /// Get the token provider
    #[allow(dead_code)]
    fn identity_provider(&self) -> P;

    /// Get the verifier
    #[allow(dead_code)]
    fn identity_verifier(&self) -> V;

    /// Get the source name
    fn source(&self) -> &Name;

    // get the session config
    fn session_config(&self) -> SessionConfig;

    // set the session config
    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError>;

    /// get the transmitter
    #[allow(dead_code)]
    fn tx(&self) -> T;

    /// get a reference to the transmitter
    #[allow(dead_code)]
    fn tx_ref(&self) -> &T;
}

#[async_trait]
pub(crate) trait MessageHandler {
    // publish a message as part of the session
    async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError>;
}
