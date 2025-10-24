// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use async_trait::async_trait;

use parking_lot::Mutex;
use parking_lot::RwLock;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::Name;
use slim_mls::mls::Mls;
use std::sync::Arc;

// Local crate
use super::SessionConfig;
use super::SessionError;
use super::SessionInterceptorProvider;
use crate::{Id, MessageDirection, common::State};

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

/// Session components lifecycle trait
/// it can be used when a session components change state
/// implements only close at the moment
pub trait SessionComponentLifecycle {
    fn close(&mut self);
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

    /// Get the destination name (point-to-point sessions only, None otherwise)
    fn dst(&self) -> Option<Name>;

    /// Get a clone of the Arc<RwLock<Option<Name>>> holding the destination
    fn dst_arc(&self) -> Arc<RwLock<Option<Name>>>;

    /// Get the mls state related to this sesson
    fn mls(&self) -> Option<Arc<Mutex<Mls<P, V>>>>;

    // get the session config
    fn session_config(&self) -> SessionConfig;

    // set the session config
    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError>;

    /// Set the destination name (point-to-point sessions). No-op for multicast.
    fn set_dst(&self, dst: Name);

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
