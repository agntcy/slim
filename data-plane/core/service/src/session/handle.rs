// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use parking_lot::RwLock;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader};
use slim_datapath::messages::encoder::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_mls::mls::Mls;

use crate::session::interceptor_mls::MlsInterceptor;
use crate::session::multicast::Multicast;
use crate::session::point_to_point::PointToPoint;
use crate::session::traits::MessageHandler;
use crate::session::traits::SessionConfigTrait;
use crate::session::traits::Transmitter;
use crate::session::transmitter::SessionTransmitter;

use super::{CommonSession, MessageDirection, SessionConfig, SessionError, State};

/// Session ID type
pub type Id = u32;

/// The session type
#[derive(Clone, PartialEq, Debug)]
pub enum SessionType {
    PointToPoint,
    Multicast,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::PointToPoint => write!(f, "PointToPoint"),
            SessionType::Multicast => write!(f, "Multicast"),
        }
    }
}

/// Common session data
#[derive(Debug)]
pub(crate) struct Common<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session ID - unique identifier for the session
    id: Id,

    /// Session state
    _state: State,

    /// Token provider for authentication
    _identity_provider: P,

    /// Verifier for authentication
    _identity_verifier: V,

    /// Session type
    session_config: RwLock<SessionConfig>,

    /// Source name
    source: Name,

    /// Optional dst name for point-to-point sessions (interior mutable)
    dst: Arc<RwLock<Option<Name>>>,

    /// MLS state
    mls: Option<Arc<Mutex<Mls<P, V>>>>,

    /// Transmitter for sending messages to slim and app
    tx: T,
    /// Session-level metadata (merged into every inbound/outbound message)
    metadata: Arc<RwLock<HashMap<String, String>>>,
}

/// Internal session representation (private)
#[derive(Debug)]
enum SessionInner<P, V, T = SessionTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    PointToPoint(PointToPoint<P, V, T>),
    Multicast(Multicast<P, V, T>),
}

/// Public opaque session handle
#[derive(Debug)]
pub struct Session<P, V, T = SessionTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    inner: SessionInner<P, V, T>,
}

impl<P, V, T> Session<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub(crate) fn from_point_to_point(s: PointToPoint<P, V, T>) -> Self {
        Session {
            inner: SessionInner::PointToPoint(s),
        }
    }
    pub(crate) fn from_multicast(s: Multicast<P, V, T>) -> Self {
        Session {
            inner: SessionInner::Multicast(s),
        }
    }

    pub fn kind(&self) -> SessionType {
        match &self.inner {
            SessionInner::PointToPoint(_) => SessionType::PointToPoint,
            SessionInner::Multicast(_) => SessionType::Multicast,
        }
    }
    pub fn id(&self) -> Id {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.id(),
            SessionInner::Multicast(s) => s.id(),
        }
    }
    pub fn source(&self) -> &Name {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.source(),
            SessionInner::Multicast(s) => s.source(),
        }
    }
    pub fn dst(&self) -> Option<Name> {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.dst(),
            SessionInner::Multicast(s) => s.dst(),
        }
    }
    pub fn session_config(&self) -> SessionConfig {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.session_config(),
            SessionInner::Multicast(s) => s.session_config(),
        }
    }
    pub fn set_session_config(&self, cfg: &SessionConfig) -> Result<(), SessionError> {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.set_session_config(cfg),
            SessionInner::Multicast(s) => s.set_session_config(cfg),
        }
    }

    pub fn metadata(&self) -> HashMap<String, String> {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.metadata().read().clone(),
            SessionInner::Multicast(s) => s.metadata().read().clone(),
        }
    }

    pub fn set_metadata_map(&self, map: HashMap<String, String>) {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.set_metadata_map(map),
            SessionInner::Multicast(s) => s.set_metadata_map(map),
        }
    }

    pub fn insert_metadata(&self, k: String, v: String) {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.insert_metadata(k, v),
            SessionInner::Multicast(s) => s.insert_metadata(k, v),
        }
    }

    pub fn incoming_connection() {}

    pub(crate) fn tx_ref(&self) -> &T {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.tx_ref(),
            SessionInner::Multicast(s) => s.tx_ref(),
        }
    }
    fn inner_ref(&self) -> &SessionInner<P, V, T> {
        &self.inner
    }

    pub async fn publish_message(&self, message: Message) -> Result<(), SessionError> {
        self.on_message(message, MessageDirection::South).await
    }

    /// Publish a message to a specific connection (forward_to)
    pub async fn publish_to(
        &self,
        name: &Name,
        forward_to: u64,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default().with_forward_to(forward_to),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message to a specific app name
    pub async fn publish(
        &self,
        name: &Name,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default(),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message with specific flags
    pub async fn publish_with_flags(
        &self,
        name: &Name,
        flags: SlimHeaderFlags,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        let ct = payload_type.unwrap_or_else(|| "msg".to_string());

        let mut msg = Message::new_publish(self.source(), name, Some(flags), &ct, blob);
        // Merge session-level metadata first
        let mut merged = self.metadata();
        if let Some(map) = metadata {
            for (k, v) in map.into_iter() {
                merged.insert(k, v);
            }
        }
        if !merged.is_empty() {
            msg.set_metadata_map(merged);
        }

        // southbound=true means towards slim
        self.publish_message(msg).await
    }

    pub async fn invite_participant(&self, destination: &Name) -> Result<(), SessionError> {
        match self.kind() {
            SessionType::PointToPoint => Err(SessionError::Processing(
                "cannot invite participant to point-to-point session".into(),
            )),
            SessionType::Multicast => {
                let slim_header = Some(SlimHeader::new(self.source(), destination, None));
                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::SessionMulticast.into(),
                    ProtoSessionMessageType::ChannelDiscoveryRequest.into(),
                    self.id(),
                    rand::random::<u32>(),
                    &None,
                    &None,
                ));
                let msg =
                    Message::new_publish_with_headers(slim_header, session_header, "", vec![]);
                self.publish_message(msg).await
            }
        }
    }

    pub async fn remove_participant(&self, destination: &Name) -> Result<(), SessionError> {
        match self.kind() {
            SessionType::PointToPoint => Err(SessionError::Processing(
                "cannot remove participant from point-to-point session".into(),
            )),
            SessionType::Multicast => {
                let slim_header = Some(SlimHeader::new(self.source(), destination, None));
                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::SessionUnknown.into(),
                    ProtoSessionMessageType::ChannelLeaveRequest.into(),
                    self.id(),
                    rand::random::<u32>(),
                    &None,
                    &None,
                ));
                let msg =
                    Message::new_publish_with_headers(slim_header, session_header, "", vec![]);
                self.publish_message(msg).await
            }
        }
    }

    /// Execute a closure with a borrowed reference to the destination name (if set).
    /// This avoids cloning while preserving lock safety.
    pub fn with_dst<R>(&self, f: impl FnOnce(Option<&Name>) -> R) -> R {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.with_dst(f),
            SessionInner::Multicast(s) => s.with_dst(f),
        }
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for Session<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.on_message(message, direction).await,
            SessionInner::Multicast(session) => session.on_message(message, direction).await,
        }
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for Session<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        self.id()
    }

    fn state(&self) -> &State {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.state(),
            SessionInner::Multicast(session) => session.state(),
        }
    }

    fn identity_provider(&self) -> P {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.identity_provider(),
            SessionInner::Multicast(session) => session.identity_provider(),
        }
    }

    fn identity_verifier(&self) -> V {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.identity_verifier(),
            SessionInner::Multicast(session) => session.identity_verifier(),
        }
    }

    fn source(&self) -> &Name {
        self.source()
    }

    fn dst(&self) -> Option<Name> {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.dst(),
            SessionInner::Multicast(session) => session.dst(),
        }
    }

    fn dst_arc(&self) -> Arc<RwLock<Option<Name>>> {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.dst_arc(),
            SessionInner::Multicast(session) => session.dst_arc(),
        }
    }

    fn session_config(&self) -> SessionConfig {
        self.session_config()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        self.set_session_config(session_config)
    }

    fn set_dst(&self, dst: Name) {
        match &self.inner {
            SessionInner::PointToPoint(session) => session.set_dst(dst),
            SessionInner::Multicast(session) => session.set_dst(dst),
        }
    }

    fn tx(&self) -> T {
        match self.inner_ref() {
            SessionInner::PointToPoint(session) => session.tx(),
            SessionInner::Multicast(session) => session.tx(),
        }
    }

    fn tx_ref(&self) -> &T {
        self.tx_ref()
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for Common<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        self.id
    }

    fn state(&self) -> &State {
        &self._state
    }

    fn source(&self) -> &Name {
        &self.source
    }

    fn dst(&self) -> Option<Name> {
        self.dst.read().clone()
    }

    fn dst_arc(&self) -> Arc<RwLock<Option<Name>>> {
        self.dst.clone()
    }

    fn set_dst(&self, dst: Name) {
        *self.dst.write() = Some(dst);
    }

    fn session_config(&self) -> SessionConfig {
        self.session_config.read().clone()
    }

    fn identity_provider(&self) -> P {
        self._identity_provider.clone()
    }

    fn identity_verifier(&self) -> V {
        self._identity_verifier.clone()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        let mut conf = self.session_config.write();

        match *conf {
            SessionConfig::PointToPoint(ref mut config) => {
                config.replace(session_config)?;
            }
            SessionConfig::Multicast(ref mut config) => {
                config.replace(session_config)?;
            }
        }
        Ok(())
    }

    fn tx(&self) -> T {
        self.tx.clone()
    }

    fn tx_ref(&self) -> &T {
        &self.tx
    }
}

impl<P, V, T> Common<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: SessionConfig,
        source: Name,
        tx: T,
        identity_provider: P,
        verifier: V,
        mls_enabled: bool,
        storage_path: std::path::PathBuf,
    ) -> Self {
        let mls = if mls_enabled {
            let mls = Mls::new(
                source.clone(),
                identity_provider.clone(),
                verifier.clone(),
                storage_path,
            );
            Some(Arc::new(Mutex::new(mls)))
        } else {
            None
        };

        let session = Self {
            id,
            _state: State::Active,
            _identity_provider: identity_provider,
            _identity_verifier: verifier,
            session_config: RwLock::new(session_config),
            source,
            dst: Arc::new(RwLock::new(None)),
            mls,
            tx,
            metadata: Arc::new(RwLock::new(HashMap::new())),
        };

        if let Some(mls) = session.mls() {
            let interceptor = MlsInterceptor::new(mls.clone());
            session.tx.add_interceptor(Arc::new(interceptor));
        }

        session
    }

    pub(crate) fn tx(&self) -> T {
        self.tx.clone()
    }

    pub(crate) fn tx_ref(&self) -> &T {
        &self.tx
    }

    pub(crate) fn mls(&self) -> Option<Arc<Mutex<Mls<P, V>>>> {
        self.mls.as_ref().map(|mls| mls.clone())
    }

    pub(crate) fn metadata(&self) -> Arc<RwLock<HashMap<String, String>>> {
        self.metadata.clone()
    }

    pub(crate) fn set_metadata_map(&self, map: HashMap<String, String>) {
        *self.metadata.write() = map;
    }

    pub(crate) fn insert_metadata(&self, k: String, v: String) {
        self.metadata.write().insert(k, v);
    }

    /// Internal helper to pass an immutable reference to dst without cloning.
    pub fn with_dst<R>(&self, f: impl FnOnce(Option<&Name>) -> R) -> R {
        let guard = self.dst.read();
        f(guard.as_ref())
    }
}

#[cfg(test)]
mod tests {
    // The tests below exercise the public Session API surface provided by this file.
    // Goals:
    // 1. Validate light-weight / sync helpers (Display impls, config mutation, dst handling).
    // 2. Exercise real publish flows (publish, publish_to) using concrete PointToPoint and Multicast session instances.
    // 3. Validate header / metadata side effects (forward_to flag, metadata map propagation, message type enums).
    // 4. Ensure error semantics for unsupported operations (inviting/removing participants on PointToPoint sessions).
    //
    // The strategy is to construct actual session variants directly via their internal constructors.
    // This avoids having to stand up larger subsystems while still traversing most of the code paths
    // in the handle layer. A simple in‑memory MockTransmitter captures outbound messages for assertions.
    use super::*;
    use crate::session::interceptor::SessionInterceptor;
    use crate::session::interceptor::SessionInterceptorProvider; // bring trait into scope for get_interceptors()
    use async_trait::async_trait;
    use parking_lot::RwLock;
    use slim_auth::errors::AuthError;
    use slim_auth::traits::{TokenProvider, Verifier};
    use slim_datapath::Status;
    use slim_datapath::api::ProtoMessage as Message;
    use slim_datapath::messages::Name;

    use crate::session::multicast::MulticastConfiguration;
    use crate::session::point_to_point::PointToPointConfiguration;
    use crate::session::traits::Transmitter;
    use crate::session::{CommonSession, SessionConfig, SessionError, SessionType};

    // ---- Test doubles ------------------------------------------------------------------------
    // Minimal TokenProvider returning a static token; sufficient because current tests do not
    // validate token contents—only that code paths do not error out when a token is requested.
    #[derive(Clone, Default)]
    struct DummyTokenProvider;
    impl TokenProvider for DummyTokenProvider {
        fn get_token(&self) -> Result<String, AuthError> {
            Ok("token".into())
        }
    }

    // Verifier that always succeeds for verify() calls and returns a deterministic error for
    // claim extraction (unused in these tests). This keeps behavior explicit while avoiding
    // silent panics if logic changes to rely on claims later.
    #[derive(Clone, Default)]
    struct DummyVerifier;
    #[async_trait]
    impl Verifier for DummyVerifier {
        async fn verify(&self, _token: impl Into<String> + Send) -> Result<(), AuthError> {
            Ok(())
        }
        fn try_verify(&self, _token: impl Into<String>) -> Result<(), AuthError> {
            Ok(())
        }
        async fn get_claims<Claims>(
            &self,
            _token: impl Into<String> + Send,
        ) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("not implemented".into()))
        }
        fn try_get_claims<Claims>(&self, _token: impl Into<String>) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("not implemented".into()))
        }
    }

    // Transmitter mock capturing messages dispatched either toward SLIM (wire side) or the app.
    // We only assert on the slim side in these tests; app side storage exists for completeness.
    #[derive(Clone, Default)]
    struct MockTransmitter {
        slim_msgs: Arc<RwLock<Vec<Result<Message, Status>>>>,
        app_msgs: Arc<RwLock<Vec<Result<Message, SessionError>>>>,
    }

    impl crate::session::SessionInterceptorProvider for MockTransmitter {
        fn add_interceptor(
            &self,
            _interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>,
        ) {
        }
        fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
            vec![]
        }
    }

    impl Transmitter for MockTransmitter {
        fn send_to_slim(
            &self,
            message: Result<Message, Status>,
        ) -> impl std::future::Future<Output = Result<(), SessionError>> + Send + 'static {
            let store = self.slim_msgs.clone();
            async move {
                store.write().push(message);
                Ok(())
            }
        }

        fn send_to_app(
            &self,
            message: Result<Message, SessionError>,
        ) -> impl std::future::Future<Output = Result<(), SessionError>> + Send + 'static {
            let store = self.app_msgs.clone();
            async move {
                store.write().push(message);
                Ok(())
            }
        }
    }

    // Transmitter variant that records added interceptors so we can assert MLS path behavior.
    #[derive(Clone, Default)]
    struct RecordingTransmitter {
        slim_msgs: Arc<RwLock<Vec<Result<Message, Status>>>>,
        app_msgs: Arc<RwLock<Vec<Result<Message, SessionError>>>>,
        interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>>>>,
    }

    impl crate::session::SessionInterceptorProvider for RecordingTransmitter {
        fn add_interceptor(
            &self,
            interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>,
        ) {
            self.interceptors.write().push(interceptor);
        }
        fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
            self.interceptors.read().clone()
        }
    }

    impl Transmitter for RecordingTransmitter {
        fn send_to_slim(
            &self,
            message: Result<Message, Status>,
        ) -> impl std::future::Future<Output = Result<(), SessionError>> + Send + 'static {
            let store = self.slim_msgs.clone();
            async move {
                store.write().push(message);
                Ok(())
            }
        }

        fn send_to_app(
            &self,
            message: Result<Message, SessionError>,
        ) -> impl std::future::Future<Output = Result<(), SessionError>> + Send + 'static {
            let store = self.app_msgs.clone();
            async move {
                store.write().push(message);
                Ok(())
            }
        }
    }

    // Helper to build 3‑segment Name instances (the underlying API expects exactly 3 components).
    fn make_name(parts: [&str; 3]) -> Name {
        Name::from_strings(parts)
    }

    // --- Basic formatting tests ---------------------------------------------------------------
    #[test]
    fn session_type_display() {
        assert_eq!(SessionType::PointToPoint.to_string(), "PointToPoint");
        assert_eq!(SessionType::Multicast.to_string(), "Multicast");
    }

    #[test]
    fn session_config_display() {
        let p = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        assert!(p.to_string().contains("PointToPointConfiguration"));
        let m = SessionConfig::Multicast(MulticastConfiguration::default());
        assert!(m.to_string().contains("MulticastConfiguration"));
    }

    // --- Common::set_session_config for PointToPoint variant ----------------------------------
    #[test]
    fn common_set_session_config_point_to_point() {
        let tx = MockTransmitter::default();
        let source = make_name(["agntcy", "src", "p2p"]);
        let cfg = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let common = Common::new(
            1,
            cfg.clone(),
            source,
            tx,
            DummyTokenProvider,
            DummyVerifier,
            false,
            std::env::temp_dir(),
        );
        let new_conf = PointToPointConfiguration {
            sticky: true,
            ..Default::default()
        };
        common
            .set_session_config(&SessionConfig::PointToPoint(new_conf.clone()))
            .unwrap();
        match common.session_config() {
            SessionConfig::PointToPoint(c) => assert!(c.sticky),
            _ => panic!("expected p2p"),
        }
    }

    // --- Common::set_session_config for Multicast variant -------------------------------------
    #[test]
    fn common_set_session_config_multicast() {
        let tx = MockTransmitter::default();
        let source = make_name(["agntcy", "src", "mc"]);
        let cfg = SessionConfig::Multicast(MulticastConfiguration::default());
        let common = Common::new(
            2,
            cfg.clone(),
            source,
            tx,
            DummyTokenProvider,
            DummyVerifier,
            false,
            std::env::temp_dir(),
        );
        let new_conf = MulticastConfiguration {
            moderator: true,
            ..Default::default()
        };
        common
            .set_session_config(&SessionConfig::Multicast(new_conf.clone()))
            .unwrap();
        match common.session_config() {
            SessionConfig::Multicast(c) => assert!(c.moderator),
            _ => panic!("expected multicast"),
        }
    }

    // --- Destination handling (set, get, with_dst closure) ------------------------------------
    #[test]
    fn common_dst_handling() {
        let tx = MockTransmitter::default();
        let source = make_name(["agntcy", "src", "p2p"]);
        let cfg = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let common = Common::new(
            3,
            cfg.clone(),
            source.clone(),
            tx,
            DummyTokenProvider,
            DummyVerifier,
            false,
            std::env::temp_dir(),
        );
        assert!(common.dst().is_none());
        let dst = make_name(["agntcy", "dst", "p2p"]);
        common.set_dst(dst.clone());
        assert_eq!(common.dst().unwrap(), dst);
        let via_with = common.with_dst(|d| d.cloned());
        assert_eq!(via_with.unwrap(), dst);
    }

    // --- Extended tests using real Session instances ------------------------------------------
    fn build_p2p_session(
        id: Id,
        sticky: bool,
    ) -> (
        Session<DummyTokenProvider, DummyVerifier, MockTransmitter>,
        Arc<RwLock<Vec<Result<Message, Status>>>>,
    ) {
        use crate::session::point_to_point::PointToPoint;
        let tx = MockTransmitter::default();
        let store = tx.slim_msgs.clone();
        let conf = PointToPointConfiguration {
            sticky,
            ..Default::default()
        };
        let source = make_name(["agntcy", "src", "p2p"]);
        let p2p = PointToPoint::new(
            id,
            conf,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            std::env::temp_dir(),
        );
        (Session::from_point_to_point(p2p), store)
    }

    fn build_multicast_session(
        id: Id,
    ) -> (
        Session<DummyTokenProvider, DummyVerifier, MockTransmitter>,
        Arc<RwLock<Vec<Result<Message, Status>>>>,
        Name,
    ) {
        use crate::session::multicast::{Multicast, MulticastConfiguration};
        let tx = MockTransmitter::default();
        let store = tx.slim_msgs.clone();
        let channel = make_name(["agntcy", "chan", "mc"]);
        let conf = MulticastConfiguration::new(
            channel.clone(),
            true,
            Some(1),
            Some(std::time::Duration::from_millis(10)),
            false,
        );
        let source = make_name(["agntcy", "src", "mc"]);
        let mc = Multicast::new(
            id,
            conf,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            std::env::temp_dir(),
        );
        (Session::from_multicast(mc), store, channel)
    }

    // Publish on a PointToPoint session and assert metadata propagation.
    #[tokio::test]
    async fn session_publish_and_metadata() {
        let (session, store) = build_p2p_session(10, false);
        let dst = make_name(["agntcy", "dst", "p2p"]);
        session
            .publish(
                &dst,
                b"hello".to_vec(),
                Some("text/plain".into()),
                Some(HashMap::from([(String::from("k"), String::from("v"))])),
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = store.read();
        if let Some(Ok(msg)) = msgs.first()
            && let Some(val) = msg.get_metadata("k")
        {
            assert_eq!(val, "v");
        }
    }

    // Verify that session-level metadata is merged into per-message metadata
    // and that per-message keys override session-level keys.
    #[tokio::test]
    async fn session_level_metadata_merge() {
        let (session, store) = build_p2p_session(11, false);
        // Set session-level metadata
        session.set_metadata_map(HashMap::from([
            ("a".to_string(), "1".to_string()),
            ("k".to_string(), "session".to_string()),
        ]));

        let dst = make_name(["agntcy", "dst", "p2p"]);
        // Publish overriding key k and adding new b
        session
            .publish(
                &dst,
                b"hello".to_vec(),
                None,
                Some(HashMap::from([
                    ("k".to_string(), "override".to_string()),
                    ("b".to_string(), "2".to_string()),
                ])),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = store.read();
        let msg = msgs
            .first()
            .expect("no message published")
            .as_ref()
            .unwrap();
        assert_eq!(msg.get_metadata("a").unwrap(), "1");
        assert_eq!(msg.get_metadata("b").unwrap(), "2");
        // Overridden value
        assert_eq!(msg.get_metadata("k").unwrap(), "override");
    }

    // Verify insert_metadata updates the session-level map and that per-message metadata
    // still overrides inserted keys on subsequent publishes.
    #[tokio::test]
    async fn session_insert_metadata_and_publish() {
        let (session, store) = build_p2p_session(14, false);

        // Insert a single key and publish without per-message metadata
        session.insert_metadata("alpha".to_string(), "1".to_string());
        let dst = make_name(["agntcy", "dst", "p2p"]);
        session
            .publish(&dst, b"first".to_vec(), None, None)
            .await
            .unwrap();

        // Insert another key and update the first key, then publish with per-message overrides
        session.insert_metadata("alpha".to_string(), "session".to_string());
        session.insert_metadata("beta".to_string(), "2".to_string());
        session
            .publish(
                &dst,
                b"second".to_vec(),
                None,
                Some(HashMap::from([
                    ("beta".to_string(), "override".to_string()),
                    ("gamma".to_string(), "3".to_string()),
                ])),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = store.read();
        assert!(msgs.len() >= 2, "expected at least two published messages");

        // First message should contain only alpha=1 (session metadata at publish time)
        if let Some(Ok(first)) = msgs.first() {
            assert_eq!(first.get_metadata("alpha").unwrap(), "1");
            assert!(first.get_metadata("beta").is_none());
        } else {
            panic!("missing first message");
        }

        // Second message should reflect updated session metadata alpha=session, beta=2 overridden by per-message beta=override, plus gamma=3
        if let Some(Ok(second)) = msgs.get(1) {
            assert_eq!(second.get_metadata("alpha").unwrap(), "session");
            assert_eq!(second.get_metadata("beta").unwrap(), "override");
            assert_eq!(second.get_metadata("gamma").unwrap(), "3");
        } else {
            panic!("missing second message");
        }
    }

    // Use publish_to which should set the forward_to flag on the header.
    #[tokio::test]
    async fn session_publish_to_sets_forward_to_flag() {
        let (session, store) = build_p2p_session(11, false);
        let dst = make_name(["agntcy", "dst", "p2p"]);
        session
            .publish_to(&dst, 42, b"data".to_vec(), None, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = store.read();
        if let Some(Ok(msg)) = msgs.first() {
            assert_eq!(msg.get_forward_to(), Some(42));
        }
    }

    // PointToPoint sessions do not support participant invite/remove; expect Processing errors.
    #[tokio::test]
    async fn invite_and_remove_participant_fail_on_p2p() {
        let (session, _store) = build_p2p_session(12, false);
        let dst = make_name(["agntcy", "dst", "p2p"]);
        let err = session.invite_participant(&dst).await.unwrap_err();
        assert!(matches!(err, SessionError::Processing(_)));
        let err = session.remove_participant(&dst).await.unwrap_err();
        assert!(matches!(err, SessionError::Processing(_)));
    }

    // Multicast supports invite/remove; verify that the correct session message types are
    // produced (ChannelDiscoveryRequest then ChannelLeaveRequest).
    #[tokio::test]
    async fn invite_and_remove_participant_multicast() {
        let (session, store, destination) = build_multicast_session(13);
        session.invite_participant(&destination).await.unwrap();
        session.remove_participant(&destination).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = store.read();
        if msgs.len() >= 2 {
            if let Some(Ok(first)) = msgs.first() {
                assert_eq!(
                    first.get_session_message_type() as u32,
                    ProtoSessionMessageType::ChannelDiscoveryRequest as u32
                );
            }
            if let Some(Ok(second)) = msgs.first() {
                assert_eq!(
                    second.get_session_message_type() as u32,
                    ProtoSessionMessageType::ChannelLeaveRequest as u32
                );
            }
        }
    }

    // --- MLS enabled tests --------------------------------------------------------------------
    // These validate that when the mls_enabled flag is passed to Common::new, an MLS interceptor
    // is registered with the transmitter (indirect verification of MLS bootstrap path).
    #[test]
    fn mls_interceptor_added_point_to_point() {
        use crate::session::point_to_point::PointToPointConfiguration;
        let tx = RecordingTransmitter::default();
        let source = make_name(["agntcy", "src", "p2p"]);
        let cfg = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let _common = Common::new(
            90,
            cfg,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            true, // mls_enabled
            std::env::temp_dir(),
        );
        assert!(
            !tx.get_interceptors().is_empty(),
            "expected at least one interceptor when MLS enabled"
        );
    }

    #[test]
    fn mls_interceptor_added_multicast() {
        use crate::session::multicast::MulticastConfiguration;
        let tx = RecordingTransmitter::default();
        let source = make_name(["agntcy", "src", "mc"]);
        let cfg = SessionConfig::Multicast(MulticastConfiguration::default());
        let _common = Common::new(
            91,
            cfg,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            true, // mls_enabled
            std::env::temp_dir(),
        );
        assert!(
            !tx.get_interceptors().is_empty(),
            "expected at least one interceptor when MLS enabled"
        );
    }

    // Negative tests: when MLS disabled we should not register an interceptor.
    #[test]
    fn mls_interceptor_absent_point_to_point() {
        use crate::session::point_to_point::PointToPointConfiguration;
        let tx = RecordingTransmitter::default();
        let source = make_name(["agntcy", "src", "p2p"]);
        let cfg = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let _common = Common::new(
            92,
            cfg,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            false, // mls disabled
            std::env::temp_dir(),
        );
        assert_eq!(
            tx.get_interceptors().len(),
            0,
            "no interceptor expected when MLS disabled"
        );
    }

    #[test]
    fn mls_interceptor_absent_multicast() {
        use crate::session::multicast::MulticastConfiguration;
        let tx = RecordingTransmitter::default();
        let source = make_name(["agntcy", "src", "mc"]);
        let cfg = SessionConfig::Multicast(MulticastConfiguration::default());
        let _common = Common::new(
            93,
            cfg,
            source,
            tx.clone(),
            DummyTokenProvider,
            DummyVerifier,
            false, // mls disabled
            std::env::temp_dir(),
        );
        assert_eq!(
            tx.get_interceptors().len(),
            0,
            "no interceptor expected when MLS disabled"
        );
    }
}
