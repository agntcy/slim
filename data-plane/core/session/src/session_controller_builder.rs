// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;
use tokio_util::sync::CancellationToken;

use crate::{
    common::SessionMessage, errors::SessionError, session_config::SessionConfig,
    session_moderator::SessionModerator, session_participant::SessionParticipant,
    transmitter::SessionTransmitter,
};

// Marker types for builder states
pub(crate) struct NotReady;
pub(crate) struct Ready;

pub(crate) struct SessionControllerBuilder<P, V, State = NotReady> {
    id: Option<u32>,
    source: Option<Name>,
    destination: Option<Name>,
    config: Option<SessionConfig>,
    identity_provider: Option<P>,
    identity_verifier: Option<V>,
    storage_path: Option<std::path::PathBuf>,
    tx: Option<SessionTransmitter>,
    tx_to_session_layer: Option<tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>>,
    cancellation_token: Option<CancellationToken>,
    _state: PhantomData<State>,
}

impl<P, V> SessionControllerBuilder<P, V, NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            source: None,
            destination: None,
            config: None,
            identity_provider: None,
            identity_verifier: None,
            storage_path: None,
            tx: None,
            tx_to_session_layer: None,
            cancellation_token: None,
            _state: PhantomData,
        }
    }

    pub(crate) fn with_id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub(crate) fn with_source(mut self, source: Name) -> Self {
        self.source = Some(source);
        self
    }

    pub(crate) fn with_destination(mut self, destination: Name) -> Self {
        self.destination = Some(destination);
        self
    }

    pub(crate) fn with_config(mut self, config: SessionConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub(crate) fn with_identity_provider(mut self, identity_provider: P) -> Self {
        self.identity_provider = Some(identity_provider);
        self
    }

    pub(crate) fn with_identity_verifier(mut self, identity_verifier: V) -> Self {
        self.identity_verifier = Some(identity_verifier);
        self
    }

    pub(crate) fn with_storage_path(mut self, storage_path: std::path::PathBuf) -> Self {
        self.storage_path = Some(storage_path);
        self
    }

    pub(crate) fn with_tx(mut self, tx: SessionTransmitter) -> Self {
        self.tx = Some(tx);
        self
    }

    pub(crate) fn with_tx_to_session_layer(
        mut self,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        self.tx_to_session_layer = Some(tx_to_session_layer);
        self
    }

    pub(crate) fn with_cancellation_token(mut self, cancellation_token: CancellationToken) -> Self {
        self.cancellation_token = Some(cancellation_token);
        self
    }

    pub(crate) fn ready(self) -> Result<SessionControllerBuilder<P, V, Ready>, SessionError> {
        // Verify all required fields are set
        if self.id.is_none()
            || self.source.is_none()
            || self.destination.is_none()
            || self.config.is_none()
            || self.identity_provider.is_none()
            || self.identity_verifier.is_none()
            || self.storage_path.is_none()
            || self.tx.is_none()
            || self.tx_to_session_layer.is_none()
            || self.cancellation_token.is_none()
        {
            return Err(SessionError::ConfigurationError(
                "Not all required fields are set in SessionControllerBuilder".to_string(),
            ));
        }

        Ok(SessionControllerBuilder {
            id: self.id,
            source: self.source,
            destination: self.destination,
            config: self.config,
            identity_provider: self.identity_provider,
            identity_verifier: self.identity_verifier,
            storage_path: self.storage_path,
            tx: self.tx,
            tx_to_session_layer: self.tx_to_session_layer,
            cancellation_token: self.cancellation_token,
            _state: PhantomData,
        })
    }
}

impl<P, V> SessionControllerBuilder<P, V, Ready>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) async fn build_participant(self) -> SessionParticipant<P, V> {
        SessionParticipant::new(
            self.id.unwrap(),
            self.source.unwrap(),
            self.destination.unwrap(),
            self.config.unwrap(),
            self.identity_provider.unwrap(),
            self.identity_verifier.unwrap(),
            self.storage_path.unwrap(),
            self.tx.unwrap(),
            self.tx_to_session_layer.unwrap(),
            self.cancellation_token.unwrap(),
        )
        .await
    }

    pub(crate) async fn build_moderator(self) -> SessionModerator<P, V> {
        SessionModerator::new(
            self.id.unwrap(),
            self.source.unwrap(),
            self.destination.unwrap(),
            self.config.unwrap(),
            self.identity_provider.unwrap(),
            self.identity_verifier.unwrap(),
            self.storage_path.unwrap(),
            self.tx.unwrap(),
            self.tx_to_session_layer.unwrap(),
            self.cancellation_token.unwrap(),
        )
        .await
    }
}
