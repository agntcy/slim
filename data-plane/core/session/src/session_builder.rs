// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;

use crate::{
    common::SessionMessage, errors::SessionError, session_config::SessionConfig,
    session_controller::SessionController, session_moderator::SessionModerator,
    session_participant::SessionParticipant, session_settings::SessionSettings,
    traits::MessageHandler, transmitter::SessionTransmitter,
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
///     .with_tx_to_session_layer(tx_channel)
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
///     .with_tx_to_session_layer(tx_channel)
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
    graceful_shutdown_timeout: Option<std::time::Duration>,
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
            graceful_shutdown_timeout: None,
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

    pub fn with_graceful_shutdown_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.graceful_shutdown_timeout = Some(timeout);
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
            graceful_shutdown_timeout: self.graceful_shutdown_timeout,
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
    pub fn build(self) -> Result<SessionController, SessionError> {
        let id = self.id.unwrap();
        let source = self.source.clone().unwrap();
        let destination = self.destination.clone().unwrap();
        let config = self.config.clone().unwrap();

        let role = if config.initiator {
            "Moderator"
        } else {
            "Participant"
        };
        tracing::debug!("Building SessionController as {}", role);

        let session_controller = if config.initiator {
            let (inner, tx, rx, settings) = self.build_session_stack(SessionModerator::new)?;
            SessionController::from_parts(
                id,
                source,
                destination,
                config.clone(),
                settings,
                tx,
                rx,
                inner,
            )
        } else {
            let (inner, tx, rx, settings) = self.build_session_stack(SessionParticipant::new)?;
            SessionController::from_parts(id, source, destination, config, settings, tx, rx, inner)
        };

        Ok(session_controller)
    }

    /// Generic helper function to build session stacks, eliminating code duplication
    /// between moderator and participant stack building.
    fn build_session_stack<W>(
        self,
        wrapper_constructor: impl FnOnce(crate::session::Session, SessionSettings<P, V>) -> W,
    ) -> Result<
        (
            W,
            tokio::sync::mpsc::Sender<SessionMessage>,
            tokio::sync::mpsc::Receiver<SessionMessage>,
            SessionSettings<P, V>,
        ),
        SessionError,
    >
    where
        W: MessageHandler,
    {
        let (tx_session, rx_session) = tokio::sync::mpsc::channel(256);

        // Create the base Session layer
        let inner = crate::session::Session::new(
            self.id.unwrap(),
            self.config.clone().unwrap(),
            &self.source.clone().unwrap(),
            self.tx.clone().unwrap(),
            tx_session.clone(),
        );

        let settings = SessionSettings {
            id: self.id.unwrap(),
            source: self.source.unwrap(),
            destination: self.destination.unwrap(),
            config: self.config.unwrap(),
            tx: self.tx.unwrap(),
            tx_session: tx_session.clone(),
            tx_to_session_layer: self.tx_to_session_layer.unwrap(),
            identity_provider: self.identity_provider.unwrap(),
            identity_verifier: self.identity_verifier.unwrap(),
            storage_path: self.storage_path.unwrap(),
            graceful_shutdown_timeout: self.graceful_shutdown_timeout,
        };

        let wrapper = wrapper_constructor(inner, settings.clone());

        Ok((wrapper, tx_session, rx_session, settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SessionError,
        test_utils::{MockTokenProvider, MockVerifier},
        transmitter::SessionTransmitter,
    };
    use slim_datapath::{api::ProtoSessionType, messages::Name};
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    fn create_test_config(initiator: bool) -> SessionConfig {
        SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator,
            metadata: HashMap::new(),
        }
    }

    fn create_test_name(prefix: &str) -> Name {
        Name::from_strings([prefix, "test", "name"]).with_id(1)
    }

    fn create_test_transmitter() -> SessionTransmitter {
        let (slim_tx, _) = mpsc::channel(10);
        let (app_tx, _) = mpsc::unbounded_channel();
        SessionTransmitter::new(slim_tx, app_tx)
    }

    #[test]
    fn test_builder_for_controller_creation() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller();
        assert!(builder.id.is_none());
        assert!(builder.source.is_none());
        assert!(builder.destination.is_none());
    }

    #[test]
    fn test_builder_for_participant_creation() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForParticipant, NotReady>::for_participant();
        assert!(builder.id.is_none());
        assert!(builder.source.is_none());
        assert!(builder.destination.is_none());
    }

    #[test]
    fn test_builder_for_moderator_creation() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForModerator, NotReady>::for_moderator();
        assert!(builder.id.is_none());
        assert!(builder.source.is_none());
        assert!(builder.destination.is_none());
    }

    #[test]
    fn test_builder_with_id() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(42);
        assert_eq!(builder.id, Some(42));
    }

    #[test]
    fn test_builder_with_source() {
        let source = create_test_name("source");
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_source(source.clone());
        assert_eq!(builder.source, Some(source));
    }

    #[test]
    fn test_builder_with_destination() {
        let destination = create_test_name("dest");
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_destination(destination.clone());
        assert_eq!(builder.destination, Some(destination));
    }

    #[test]
    fn test_builder_with_config() {
        let config = create_test_config(true);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config.clone());
        assert!(builder.config.is_some());
        assert!(builder.config.unwrap().initiator);
    }

    #[test]
    fn test_builder_with_identity_provider() {
        let provider = MockTokenProvider;
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_identity_provider(provider);
        assert!(builder.identity_provider.is_some());
    }

    #[test]
    fn test_builder_with_identity_verifier() {
        let verifier = MockVerifier;
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_identity_verifier(verifier);
        assert!(builder.identity_verifier.is_some());
    }

    #[test]
    fn test_builder_with_storage_path() {
        let path = std::path::PathBuf::from("/tmp/test");
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_storage_path(path.clone());
        assert_eq!(builder.storage_path, Some(path));
    }

    #[test]
    fn test_builder_with_tx() {
        let tx = create_test_transmitter();
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_tx(tx);
        assert!(builder.tx.is_some());
    }

    #[test]
    fn test_builder_with_tx_to_session_layer() {
        let (tx, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_tx_to_session_layer(tx);
        assert!(builder.tx_to_session_layer.is_some());
    }

    #[test]
    fn test_builder_ready_with_all_fields() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_ok());
    }

    #[test]
    fn test_builder_ready_missing_id() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
        match ready_result {
            Err(SessionError::ConfigurationError(msg)) => {
                assert!(msg.contains("Not all required fields are set"));
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_builder_ready_missing_source() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);
        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_destination() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_config() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_identity_provider() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_identity_verifier() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_storage_path() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_tx() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_tx_to_session_layer(tx_to_session);

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_ready_missing_tx_to_session_layer() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter());

        let ready_result = builder.ready();
        assert!(ready_result.is_err());
    }

    #[test]
    fn test_builder_chaining() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(123)
                .with_source(create_test_name("src"))
                .with_destination(create_test_name("dst"))
                .with_config(create_test_config(false))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/test/path"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        assert_eq!(builder.id, Some(123));
        assert!(builder.source.is_some());
        assert!(builder.destination.is_some());
        assert!(builder.config.is_some());
        assert!(builder.identity_provider.is_some());
        assert!(builder.identity_verifier.is_some());
        assert_eq!(
            builder.storage_path,
            Some(std::path::PathBuf::from("/test/path"))
        );
        assert!(builder.tx.is_some());
        assert!(builder.tx_to_session_layer.is_some());
    }

    #[test]
    fn test_builder_ready_state_transition() {
        let (tx_to_session, _) = mpsc::channel(10);
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(std::path::PathBuf::from("/tmp"))
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let ready_builder = builder.ready().unwrap();

        // Verify all fields are still present after transition
        assert_eq!(ready_builder.id, Some(1));
        assert!(ready_builder.source.is_some());
        assert!(ready_builder.destination.is_some());
        assert!(ready_builder.config.is_some());
        assert!(ready_builder.identity_provider.is_some());
        assert!(ready_builder.identity_verifier.is_some());
        assert!(ready_builder.storage_path.is_some());
        assert!(ready_builder.tx.is_some());
        assert!(ready_builder.tx_to_session_layer.is_some());
    }

    #[test]
    fn test_builder_different_target_types() {
        // Test that different target types can be created independently
        let _controller_builder = SessionBuilder::<
            MockTokenProvider,
            MockVerifier,
            ForController,
            NotReady,
        >::for_controller();
        let _participant_builder = SessionBuilder::<
            MockTokenProvider,
            MockVerifier,
            ForParticipant,
            NotReady,
        >::for_participant();
        let _moderator_builder = SessionBuilder::<
            MockTokenProvider,
            MockVerifier,
            ForModerator,
            NotReady,
        >::for_moderator();
    }

    #[test]
    fn test_builder_with_different_config_types() {
        let config_initiator = create_test_config(true);
        let config_participant = create_test_config(false);

        let builder1 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config_initiator);
        assert!(builder1.config.unwrap().initiator);

        let builder2 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config_participant);
        assert!(!builder2.config.unwrap().initiator);
    }

    #[test]
    fn test_builder_with_multicast_session_config() {
        let mut config = create_test_config(true);
        config.session_type = ProtoSessionType::Multicast;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        assert_eq!(
            builder.config.unwrap().session_type,
            ProtoSessionType::Multicast
        );
    }

    #[test]
    fn test_builder_with_mls_enabled() {
        let mut config = create_test_config(true);
        config.mls_enabled = true;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        assert!(builder.config.unwrap().mls_enabled);
    }

    #[test]
    fn test_builder_with_custom_retry_settings() {
        let mut config = create_test_config(true);
        config.max_retries = Some(10);
        config.interval = Some(std::time::Duration::from_secs(5));

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config.clone());

        let stored_config = builder.config.unwrap();
        assert_eq!(stored_config.max_retries, Some(10));
        assert_eq!(
            stored_config.interval,
            Some(std::time::Duration::from_secs(5))
        );
    }

    #[test]
    fn test_builder_with_metadata() {
        let mut config = create_test_config(true);
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());
        config.metadata = metadata.clone();

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        let stored_config = builder.config.unwrap();
        assert_eq!(
            stored_config.metadata.get("key1"),
            Some(&"value1".to_string())
        );
        assert_eq!(
            stored_config.metadata.get("key2"),
            Some(&"value2".to_string())
        );
    }

    #[test]
    fn test_builder_overwrites_previous_values() {
        // Test that calling builder methods multiple times overwrites previous values
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_id(2)
                .with_id(3);

        assert_eq!(builder.id, Some(3));

        let source1 = create_test_name("first");
        let source2 = create_test_name("second");
        let builder = SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
            .with_source(source1)
            .with_source(source2.clone());

        assert_eq!(builder.source, Some(source2));
    }

    #[test]
    fn test_builder_partial_configuration() {
        // Test that builder can be created with partial configuration
        // and then completed later
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(42);

        assert_eq!(builder.id, Some(42));
        assert!(builder.source.is_none());

        // Continue building
        let (tx_to_session, _) = mpsc::channel(10);
        let builder = builder
            .with_source(create_test_name("source"))
            .with_destination(create_test_name("dest"))
            .with_config(create_test_config(true))
            .with_identity_provider(MockTokenProvider)
            .with_identity_verifier(MockVerifier)
            .with_storage_path(std::path::PathBuf::from("/tmp"))
            .with_tx(create_test_transmitter())
            .with_tx_to_session_layer(tx_to_session);

        assert!(builder.ready().is_ok());
    }

    #[test]
    fn test_builder_ready_validation_comprehensive() {
        // Test that ready() properly validates each field individually
        let (tx_to_session, _) = mpsc::channel(10);

        // Create a fully configured builder
        let mut builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller();

        builder.id = Some(1);
        builder.source = Some(create_test_name("source"));
        builder.destination = Some(create_test_name("dest"));
        builder.config = Some(create_test_config(true));
        builder.identity_provider = Some(MockTokenProvider);
        builder.identity_verifier = Some(MockVerifier);
        builder.storage_path = Some(std::path::PathBuf::from("/tmp"));
        builder.tx = Some(create_test_transmitter());
        builder.tx_to_session_layer = Some(tx_to_session);

        // This should succeed
        assert!(builder.ready().is_ok());
    }

    #[test]
    fn test_builder_with_empty_metadata() {
        let config = create_test_config(true);
        assert!(config.metadata.is_empty());

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        assert!(builder.config.unwrap().metadata.is_empty());
    }

    #[test]
    fn test_builder_with_none_retries() {
        let mut config = create_test_config(true);
        config.max_retries = None;
        config.interval = None;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        let stored = builder.config.unwrap();
        assert_eq!(stored.max_retries, None);
        assert_eq!(stored.interval, None);
    }

    #[test]
    fn test_builder_with_zero_duration() {
        let mut config = create_test_config(true);
        config.interval = Some(std::time::Duration::from_secs(0));

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config);

        assert_eq!(
            builder.config.unwrap().interval,
            Some(std::time::Duration::from_secs(0))
        );
    }

    #[test]
    fn test_builder_with_large_id() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(u32::MAX);

        assert_eq!(builder.id, Some(u32::MAX));
    }

    #[test]
    fn test_builder_type_states() {
        // Verify that NotReady state has the methods we expect
        let not_ready =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller();

        // Can call builder methods
        let not_ready = not_ready.with_id(1);
        assert_eq!(not_ready.id, Some(1));

        // PhantomData doesn't affect size
        assert_eq!(
            std::mem::size_of::<
                SessionBuilder<MockTokenProvider, MockVerifier, ForController, NotReady>,
            >(),
            std::mem::size_of::<
                SessionBuilder<MockTokenProvider, MockVerifier, ForController, Ready>,
            >()
        );
    }

    #[test]
    fn test_builder_clone_safe_types() {
        // Test that clone-safe types work correctly
        let provider1 = MockTokenProvider;
        let provider2 = provider1.clone();

        let builder1 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_identity_provider(provider1);

        let builder2 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_identity_provider(provider2);

        assert!(builder1.identity_provider.is_some());
        assert!(builder2.identity_provider.is_some());
    }

    #[test]
    fn test_builder_error_message_content() {
        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1);

        match builder.ready() {
            Err(SessionError::ConfigurationError(msg)) => {
                assert!(msg.contains("Not all required fields"));
                assert!(msg.contains("SessionBuilder"));
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_builder_with_all_session_types() {
        // Test with PointToPoint
        let config_p2p = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: None,
            interval: None,
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        };

        let builder_p2p =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_config(config_p2p);
        assert_eq!(
            builder_p2p.config.unwrap().session_type,
            ProtoSessionType::PointToPoint
        );

        // Test with Multicast
        let config_multicast = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: None,
            interval: None,
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        };

        let builder_multicast = SessionBuilder::<
            MockTokenProvider,
            MockVerifier,
            ForController,
            NotReady,
        >::for_controller()
        .with_config(config_multicast);
        assert_eq!(
            builder_multicast.config.unwrap().session_type,
            ProtoSessionType::Multicast
        );

        // Test with Unspecified
        let config_unspecified = SessionConfig {
            session_type: ProtoSessionType::Unspecified,
            max_retries: None,
            interval: None,
            mls_enabled: false,
            initiator: false,
            metadata: HashMap::new(),
        };

        let builder_unspecified = SessionBuilder::<
            MockTokenProvider,
            MockVerifier,
            ForController,
            NotReady,
        >::for_controller()
        .with_config(config_unspecified);
        assert_eq!(
            builder_unspecified.config.unwrap().session_type,
            ProtoSessionType::Unspecified
        );
    }

    // ========== ASYNC TESTS ==========

    #[tokio::test]
    async fn test_builder_build_as_participant() {
        // Create temporary directory for MLS storage
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _rx_from_session) = mpsc::channel(10);
        let config = create_test_config(false); // participant (not initiator)

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(1)
                .with_source(create_test_name("participant"))
                .with_destination(create_test_name("moderator"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();

        // Should succeed for participant
        assert!(controller.is_ok());
        let controller = controller.unwrap();
        assert_eq!(controller.id(), 1);
        assert!(!controller.is_initiator());

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_as_moderator_p2p() {
        // Create temporary directory for MLS storage
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _rx_from_session) = mpsc::channel(10);
        let (slim_tx, mut slim_rx) = mpsc::channel(10);
        let (app_tx, _app_rx) = mpsc::unbounded_channel();
        let tx = SessionTransmitter::new(slim_tx, app_tx);

        let mut config = create_test_config(true); // moderator (initiator)
        config.session_type = ProtoSessionType::PointToPoint;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(2)
                .with_source(create_test_name("moderator"))
                .with_destination(create_test_name("participant"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(tx)
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();

        assert!(controller.is_ok());
        let controller = controller.unwrap();
        assert_eq!(controller.id(), 2);
        assert!(controller.is_initiator());
        assert_eq!(controller.session_type(), ProtoSessionType::PointToPoint);

        // For P2P initiator, should send discovery request
        // Check that a message was sent to slim
        tokio::time::timeout(std::time::Duration::from_millis(100), slim_rx.recv())
            .await
            .ok();

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_as_moderator_multicast() {
        // Create temporary directory for MLS storage
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _rx_from_session) = mpsc::channel(10);
        let (slim_tx, mut slim_rx) = mpsc::channel(10);
        let (app_tx, _app_rx) = mpsc::unbounded_channel();
        let tx = SessionTransmitter::new(slim_tx, app_tx);

        let mut config = create_test_config(true); // moderator (initiator)
        config.session_type = ProtoSessionType::Multicast;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(3)
                .with_source(create_test_name("moderator"))
                .with_destination(create_test_name("group"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(tx)
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();

        assert!(controller.is_ok());
        let controller = controller.unwrap();
        assert_eq!(controller.id(), 3);
        assert!(controller.is_initiator());
        assert_eq!(controller.session_type(), ProtoSessionType::Multicast);

        // For Multicast, should NOT send discovery request automatically
        // Try to receive with timeout - should timeout (no message)
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), slim_rx.recv()).await;
        // Either timeout or channel is empty
        assert!(result.is_err() || result.unwrap().is_none());

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_with_mls_disabled() {
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _) = mpsc::channel(10);
        let mut config = create_test_config(false);
        config.mls_enabled = false;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(4)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();
        assert!(controller.is_ok());

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_with_custom_retry_settings() {
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _) = mpsc::channel(10);
        let mut config = create_test_config(true);
        config.max_retries = Some(5);
        config.interval = Some(std::time::Duration::from_millis(500));

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(5)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(config.clone())
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();
        assert!(controller.is_ok());

        let controller = controller.unwrap();
        let retrieved_config = controller.session_config();
        assert_eq!(retrieved_config.max_retries, Some(5));
        assert_eq!(
            retrieved_config.interval,
            Some(std::time::Duration::from_millis(500))
        );

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_with_metadata() {
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _) = mpsc::channel(10);
        let mut config = create_test_config(false);
        let mut metadata = HashMap::new();
        metadata.insert("app_name".to_string(), "test_app".to_string());
        metadata.insert("version".to_string(), "1.0".to_string());
        config.metadata = metadata.clone();

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(6)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();
        assert!(controller.is_ok());

        let controller = controller.unwrap();
        let retrieved_metadata = controller.metadata();
        assert_eq!(
            retrieved_metadata.get("app_name"),
            Some(&"test_app".to_string())
        );
        assert_eq!(retrieved_metadata.get("version"), Some(&"1.0".to_string()));

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_verifies_session_source_and_destination() {
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _) = mpsc::channel(10);
        let source = create_test_name("my_source");
        let destination = create_test_name("my_dest");

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(7)
                .with_source(source.clone())
                .with_destination(destination.clone())
                .with_config(create_test_config(false))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();
        assert!(controller.is_ok());

        let controller = controller.unwrap();
        assert_eq!(controller.source(), &source);
        assert_eq!(controller.dst(), &destination);

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_build_multiple_sessions_different_ids() {
        let temp_dir1 =
            std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        let temp_dir2 =
            std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir1).unwrap();
        std::fs::create_dir_all(&temp_dir2).unwrap();

        let (tx_to_session1, _) = mpsc::channel(10);
        let (tx_to_session2, _) = mpsc::channel(10);

        let builder1 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(100)
                .with_source(create_test_name("source1"))
                .with_destination(create_test_name("dest1"))
                .with_config(create_test_config(false))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir1.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session1);

        let builder2 =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(200)
                .with_source(create_test_name("source2"))
                .with_destination(create_test_name("dest2"))
                .with_config(create_test_config(true))
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir2.clone())
                .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session2);

        let controller1 = builder1.ready().unwrap().build();
        let controller2 = builder2.ready().unwrap().build();

        assert!(controller1.is_ok());
        assert!(controller2.is_ok());

        let controller1 = controller1.unwrap();
        let controller2 = controller2.unwrap();

        assert_eq!(controller1.id(), 100);
        assert_eq!(controller2.id(), 200);
        assert_ne!(controller1.id(), controller2.id());

        // Cleanup
        drop(controller1);
        drop(controller2);
        let _ = std::fs::remove_dir_all(&temp_dir1);
        let _ = std::fs::remove_dir_all(&temp_dir2);
    }

    #[tokio::test]
    async fn test_builder_build_with_unspecified_session_type() {
        let temp_dir = std::env::temp_dir().join(format!("test_session_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (tx_to_session, _) = mpsc::channel(10);
        let mut config = create_test_config(false);
        config.session_type = ProtoSessionType::Unspecified;

        let builder =
            SessionBuilder::<MockTokenProvider, MockVerifier, ForController, NotReady>::for_controller()
                .with_id(8)
                .with_source(create_test_name("source"))
                .with_destination(create_test_name("dest"))
                .with_config(config)
                .with_identity_provider(MockTokenProvider)
                .with_identity_verifier(MockVerifier)
                .with_storage_path(temp_dir.clone())
               .with_tx(create_test_transmitter())
                .with_tx_to_session_layer(tx_to_session);

        let controller = builder.ready().unwrap().build();
        assert!(controller.is_ok());

        let controller = controller.unwrap();
        assert_eq!(controller.session_type(), ProtoSessionType::Unspecified);

        // Cleanup
        drop(controller);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
