// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use async_trait::async_trait;
use parking_lot::Mutex;
use parking_lot::RwLock;
use tonic::Status;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::encoder::Name;
use slim_mls::mls::Mls;

// Local crate
use crate::session::CommonSession;
use crate::session::Id;
use crate::session::SessionConfig;
use crate::session::SessionError;
use crate::session::interceptor_mls::MlsInterceptor;
use crate::session::traits::SessionConfigTrait;
use crate::session::traits::Transmitter;

/// Reserved session id
pub const SESSION_RANGE: std::ops::Range<u32> = 0..(u32::MAX - 1000);

/// Unspecified session ID constant
pub const SESSION_UNSPECIFIED: u32 = u32::MAX;

/// Channel used in the path service -> app
pub(crate) type AppChannelSender = tokio::sync::mpsc::Sender<Result<Message, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::Receiver<Result<Message, SessionError>>;
/// Channel used in the path service -> slim
pub(crate) type SlimChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
#[allow(dead_code)]
pub enum State {
    Active,
    Inactive,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum MessageDirection {
    North,
    South,
}

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
    #[allow(dead_code)]
    id: Id,

    /// Session state
    #[allow(dead_code)]
    state: State,

    /// Token provider for authentication
    #[allow(dead_code)]
    identity_provider: P,

    /// Verifier for authentication
    #[allow(dead_code)]
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

// Session implementation moved to handle.rs

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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn tx_ref(&self) -> &T {
        &self.tx
    }

    pub(crate) fn mls(&self) -> Option<Arc<Mutex<Mls<P, V>>>> {
        self.mls.as_ref().map(|mls| mls.clone())
    }
}
