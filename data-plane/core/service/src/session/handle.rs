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

use crate::session::multicast::Multicast;
use crate::session::point_to_point::PointToPoint;
use crate::session::traits::MessageHandler;
use crate::session::traits::Transmitter;
use crate::session::transmitter::SessionTransmitter;
use crate::session::interceptor_mls::MlsInterceptor;
use crate::session::traits::SessionConfigTrait;

use super::{CommonSession, Id, MessageDirection, SessionConfig, SessionError, State};

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
    state: State,

    /// Token provider for authentication
    identity_provider: P,

    /// Verifier for authentication
    identity_verifier: V,

    /// Session type
    session_config: RwLock<SessionConfig>,

    /// Source name
    source: Name,

    /// MLS state (used only in pub/sub section for the moment)
    mls: Option<Arc<Mutex<Mls<P, V>>>>,

    /// Transmitter for sending messages to slim and app
    tx: T,
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
    pub(crate) fn tx_ref(&self) -> &T {
        match &self.inner {
            SessionInner::PointToPoint(s) => s.tx_ref(),
            SessionInner::Multicast(s) => s.tx_ref(),
        }
    }
    fn inner_ref(&self) -> &SessionInner<P, V, T> {
        &self.inner
    }

    pub async fn publish_message(
        &self,
        message: Message,
    ) -> Result<(), SessionError> {
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

        if let Some(map) = metadata {
            msg.set_metadata_map(map);
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

    fn state(&self) -> &super::State {
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

    fn session_config(&self) -> SessionConfig {
        self.session_config()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        self.set_session_config(session_config)
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
        &self.state
    }

    fn source(&self) -> &Name {
        &self.source
    }

    fn session_config(&self) -> SessionConfig {
        self.session_config.read().clone()
    }

    fn identity_provider(&self) -> P {
        self.identity_provider.clone()
    }

    fn identity_verifier(&self) -> V {
        self.identity_verifier.clone()
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
            state: State::Active,
            identity_provider,
            identity_verifier: verifier,
            session_config: RwLock::new(session_config),
            source,
            mls,
            tx,
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
}
