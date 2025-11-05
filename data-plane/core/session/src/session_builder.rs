// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType,
        SessionHeader, SlimHeader,
    },
    messages::Name,
};
use tokio_util::sync::CancellationToken;

use crate::{
    MessageDirection,
    common::SessionMessage,
    errors::SessionError,
    session_config::SessionConfig,
    session_controller::{SessionController, SessionControllerImpl},
    session_moderator::SessionModerator,
    session_participant::SessionParticipant,
    session_settings::SessionSettings,
    transmitter::SessionTransmitter,
};

// Marker types for builder states
pub struct NotReady;
pub struct Ready;

// Marker types for target types
pub struct ForController;
pub struct ForParticipant;
pub struct ForModerator;

/// Unified generic builder for constructing session-related types.
///
/// This builder eliminates the need for multiple constructors with many parameters
/// by providing a fluent, type-safe API for building session components.
///
/// # Type Safety
///
/// The builder uses compile-time type states to ensure:
/// 1. All required fields are set before building (enforced by `ready()`)
/// 2. The correct build method is available for each target type
/// 3. No runtime panics due to missing fields
///
/// # Supported Types
///
/// - **`SessionController`** - High-level controller that wraps participant/moderator
///   - Use `SessionBuilder::for_controller()` or `SessionController::builder()`
///   - Automatically creates moderator or participant based on config
///
/// - **`SessionParticipant`** - Direct participant construction (advanced)
///   - Use `SessionBuilder::for_participant()` or `SessionParticipant::builder()`
///
/// - **`SessionModerator`** - Direct moderator construction (advanced)
///   - Use `SessionBuilder::for_moderator()` or `SessionModerator::builder()`
///
/// # Examples
///
/// ## Building a SessionController (Recommended)
///
/// ```ignore
/// use agntcy_slim_session::{SessionBuilder, SessionController};
///
/// let controller = SessionController::builder()
///     .with_id(1)
///     .with_source(source_name)
///     .with_destination(dest_name)
///     .with_config(session_config)
///     .with_identity_provider(provider)
///     .with_identity_verifier(verifier)
///     .with_storage_path(storage_path)
///     .with_tx(transmitter)
///     .with_tx_to_session_layer(tx_channel)
///     .with_cancellation_token(token)
///     .ready()?  // Validates all fields are set
///     .build()   // Constructs the SessionController
///     .await?;
/// ```
///
/// ## Building a SessionParticipant (Advanced)
///
/// ```ignore
/// let participant = SessionBuilder::for_participant()
///     .with_id(1)
///     .with_source(name)
///     .with_destination(dest)
///     .with_config(config)
///     .with_identity_provider(provider)
///     .with_identity_verifier(verifier)
///     .with_storage_path(path)
///     .with_tx(tx)
///     .with_tx_to_session_layer(tx_layer)
///     .with_cancellation_token(token)
///     .ready()?
///     .build()
///     .await;
/// ```
///
/// ## Building a SessionModerator (Advanced)
///
/// ```ignore
/// let moderator = SessionModerator::builder()
///     .with_id(session_id)
///     .with_source(moderator_name)
///     .with_destination(group_name)
///     .with_config(moderator_config)
///     .with_identity_provider(provider)
///     .with_identity_verifier(verifier)
///     .with_storage_path(storage_path)
///     .with_tx(tx)
///     .with_tx_to_session_layer(tx_to_layer)
///     .with_cancellation_token(cancel_token)
///     .ready()?
///     .build()
///     .await;
/// ```
pub struct SessionBuilder<P, V, Target, State = NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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
    _target: PhantomData<Target>,
    _state: PhantomData<State>,
}

// Common builder methods (available in NotReady state for all target types)
impl<P, V, Target> SessionBuilder<P, V, Target, NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn new() -> Self {
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
            _target: PhantomData,
            _state: PhantomData,
        }
    }

    pub fn with_id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_source(mut self, source: Name) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_destination(mut self, destination: Name) -> Self {
        self.destination = Some(destination);
        self
    }

    pub fn with_config(mut self, config: SessionConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_identity_provider(mut self, identity_provider: P) -> Self {
        self.identity_provider = Some(identity_provider);
        self
    }

    pub fn with_identity_verifier(mut self, identity_verifier: V) -> Self {
        self.identity_verifier = Some(identity_verifier);
        self
    }

    pub fn with_storage_path(mut self, storage_path: std::path::PathBuf) -> Self {
        self.storage_path = Some(storage_path);
        self
    }

    pub fn with_tx(mut self, tx: SessionTransmitter) -> Self {
        self.tx = Some(tx);
        self
    }

    pub fn with_tx_to_session_layer(
        mut self,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        self.tx_to_session_layer = Some(tx_to_session_layer);
        self
    }

    pub fn with_cancellation_token(mut self, cancellation_token: CancellationToken) -> Self {
        self.cancellation_token = Some(cancellation_token);
        self
    }

    pub fn ready(self) -> Result<SessionBuilder<P, V, Target, Ready>, SessionError> {
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
                "Not all required fields are set in SessionBuilder".to_string(),
            ));
        }

        Ok(SessionBuilder {
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
            _target: PhantomData,
            _state: PhantomData,
        })
    }
}

// Convenience constructors for different target types
impl<P, V> SessionBuilder<P, V, ForController, NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new builder for constructing a SessionController
    pub fn for_controller() -> Self {
        Self::new()
    }
}

impl<P, V> SessionBuilder<P, V, ForParticipant, NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new builder for constructing a SessionParticipant
    pub fn for_participant() -> Self {
        Self::new()
    }
}

impl<P, V> SessionBuilder<P, V, ForModerator, NotReady>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new builder for constructing a SessionModerator
    pub fn for_moderator() -> Self {
        Self::new()
    }
}

// Build methods for SessionController
impl<P, V> SessionBuilder<P, V, ForController, Ready>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Build a SessionController
    ///
    /// Automatically determines whether to create a moderator or participant
    /// internally based on the session configuration's `initiator` flag.
    pub async fn build(self) -> Result<SessionController, SessionError> {
        let id = self.id.unwrap();
        let source = self.source.clone().unwrap();
        let destination = self.destination.clone().unwrap();
        let config = self.config.clone().unwrap();
        let cancellation_token = self.cancellation_token.clone().unwrap();

        let controller = if config.initiator {
            SessionControllerImpl::SessionModerator(self.build_moderator_impl().await)
        } else {
            SessionControllerImpl::SessionParticipant(self.build_participant_impl().await)
        };

        let session_controller = SessionController::from_parts(
            id,
            source,
            destination,
            config,
            controller,
            cancellation_token,
        );

        // if the session is a p2p session and the session is created
        // as initiator we need to invite the remote participant
        if session_controller.is_initiator()
            && session_controller.session_type() == ProtoSessionType::PointToPoint
        {
            // send a discovery request
            let slim_header = Some(SlimHeader::new(
                session_controller.source(),
                session_controller.dst(),
                "",
                None,
            ));
            let session_header = Some(SessionHeader::new(
                session_controller.session_type().into(),
                ProtoSessionMessageType::DiscoveryRequest.into(),
                session_controller.id(),
                rand::random::<u32>(),
            ));
            let payload = Some(CommandPayload::new_discovery_request_payload(None).as_content());
            let discovery = Message::new_publish_with_headers(slim_header, session_header, payload);
            session_controller
                .on_message(discovery, MessageDirection::South)
                .await?;
        }

        Ok(session_controller)
    }

    async fn build_moderator_impl(self) -> SessionModerator<P, V> {
        let settings = SessionSettings {
            id: self.id.unwrap(),
            source: self.source.unwrap(),
            destination: self.destination.unwrap(),
            config: self.config.unwrap(),
            identity_provider: self.identity_provider.unwrap(),
            identity_verifier: self.identity_verifier.unwrap(),
            storage_path: self.storage_path.unwrap(),
            tx: self.tx.unwrap(),
            tx_to_session_layer: self.tx_to_session_layer.unwrap(),
        };
        SessionModerator::new(settings).await
    }

    async fn build_participant_impl(self) -> SessionParticipant<P, V> {
        let settings = SessionSettings {
            id: self.id.unwrap(),
            source: self.source.unwrap(),
            destination: self.destination.unwrap(),
            config: self.config.unwrap(),
            identity_provider: self.identity_provider.unwrap(),
            identity_verifier: self.identity_verifier.unwrap(),
            storage_path: self.storage_path.unwrap(),
            tx: self.tx.unwrap(),
            tx_to_session_layer: self.tx_to_session_layer.unwrap(),
        };
        SessionParticipant::new(settings).await
    }
}

// Build methods for SessionParticipant
impl<P, V> SessionBuilder<P, V, ForParticipant, Ready>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Build a SessionParticipant
    pub async fn build(self) -> SessionParticipant<P, V> {
        let settings = SessionSettings {
            id: self.id.unwrap(),
            source: self.source.unwrap(),
            destination: self.destination.unwrap(),
            config: self.config.unwrap(),
            identity_provider: self.identity_provider.unwrap(),
            identity_verifier: self.identity_verifier.unwrap(),
            storage_path: self.storage_path.unwrap(),
            tx: self.tx.unwrap(),
            tx_to_session_layer: self.tx_to_session_layer.unwrap(),
        };
        SessionParticipant::new(settings).await
    }
}

// Build methods for SessionModerator
impl<P, V> SessionBuilder<P, V, ForModerator, Ready>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Build a SessionModerator
    pub async fn build(self) -> SessionModerator<P, V> {
        let settings = SessionSettings {
            id: self.id.unwrap(),
            source: self.source.unwrap(),
            destination: self.destination.unwrap(),
            config: self.config.unwrap(),
            identity_provider: self.identity_provider.unwrap(),
            identity_verifier: self.identity_verifier.unwrap(),
            storage_path: self.storage_path.unwrap(),
            tx: self.tx.unwrap(),
            tx_to_session_layer: self.tx_to_session_layer.unwrap(),
        };
        SessionModerator::new(settings).await
    }
}
