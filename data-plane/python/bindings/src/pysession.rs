// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::exceptions::PyException;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyclass_enum;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use slim_service::{AppChannelReceiver, SessionError};
// (Python-only session wrapper will provide higher-level methods; keep Rust minimal)

use crate::pyidentity::{IdentityProvider, IdentityVerifier};
use crate::utils::PyName;
use slim_service::MulticastConfiguration;
use slim_service::PointToPointConfiguration;
use slim_service::session;
pub use slim_service::session::SESSION_UNSPECIFIED;
use slim_service::session::Session;
use slim_service::session::context::SessionContext;

/// Internal shared session context state.
///
/// Holds:
/// * a weak reference to the underlying `Session` (so that Python
///   references do not keep a closed session alive),
/// * a receiver (`rx`) for application/channel messages which is
///   protected by an async `RwLock` to allow concurrent access patterns.
///
/// This struct is not exposed directly to Python; it is wrapped by
/// `PySessionContext`.
pub(crate) struct PySessionCtxInternal {
    pub(crate) session: Weak<Session<IdentityProvider, IdentityVerifier>>,
    pub(crate) rx: RwLock<AppChannelReceiver>,
}

/// Python-exposed session context wrapper.
///
/// A thin, cloneable handle around the underlying Rust session state. All
/// getters perform a safe upgrade of the weak internal session reference,
/// returning a Python exception if the session has already been closed.
/// The internal message receiver is intentionally not exposed at this level.
///
/// Higher-level Python code (see `session.py`) provides ergonomic async
/// operations on top of this context.
///
/// Properties (getters exposed to Python):
/// - id -> int: Unique numeric identifier of the session. Raises a Python
///   exception if the session has been closed.
/// - metadata -> dict[str,str]: Arbitrary key/value metadata copied from the
///   current SessionConfig. A cloned map is returned so Python can mutate
///   without racing the underlying config.
/// - session_type -> PySessionType: High-level transport classification
///   (P2P, GROUP), inferred from internal kind + destination.
/// - src -> PyName: Fully qualified source identity that originated / owns
///   the session.
/// - dst -> PyName: Destination name:
///     * PyName of the peer for P2P
///     * PyName of the channel for GROUP
/// - session_config -> PySessionConfiguration: Current effective configuration
///   converted to the Python-facing enum variant.
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PySessionContext {
    pub(crate) internal: Arc<PySessionCtxInternal>,
}

impl From<SessionContext<IdentityProvider, IdentityVerifier>> for PySessionContext {
    fn from(ctx: SessionContext<IdentityProvider, IdentityVerifier>) -> Self {
        // Split context into constituent parts (session + channel receiver)
        let (session, rx) = ctx.into_parts();
        let rx = RwLock::new(rx);

        PySessionContext {
            internal: Arc::new(PySessionCtxInternal { session, rx }),
        }
    }
}

// Internal helper to obtain a strong session reference or raise a Python exception
fn strong_session(
    weak: &Weak<Session<IdentityProvider, IdentityVerifier>>,
) -> PyResult<Arc<Session<IdentityProvider, IdentityVerifier>>> {
    weak.upgrade().ok_or_else(|| {
        PyErr::new::<PyException, _>(
            SessionError::SessionClosed("session already closed".to_string()).to_string(),
        )
    })
}

#[gen_stub_pymethods]
#[pymethods]
impl PySessionContext {
    #[getter]
    pub fn id(&self) -> PyResult<u32> {
        let id = strong_session(&self.internal.session)?.id();

        Ok(id)
    }

    #[getter]
    pub fn metadata(&self) -> PyResult<HashMap<String, String>> {
        let session = self.internal.session.upgrade().ok_or_else(|| {
            PyErr::new::<PyException, _>(
                SessionError::SessionClosed("session already closed".to_string()).to_string(),
            )
        })?;
        let session_config = session.session_config();

        Ok(session_config.metadata())
    }

    #[getter]
    pub fn session_type(&self) -> PyResult<PySessionType> {
        let session = strong_session(&self.internal.session)?;
        Ok(session.kind().into())
    }

    #[getter]
    pub fn src(&self) -> PyResult<PyName> {
        let session = strong_session(&self.internal.session)?;

        Ok(session.source().clone().into())
    }

    #[getter]
    pub fn dst(&self) -> PyResult<Option<PyName>> {
        let session = strong_session(&self.internal.session)?;

        Ok(session.dst().map(|name| name.into()))
    }

    #[getter]
    pub fn session_config(&self) -> PyResult<PySessionConfiguration> {
        let session = strong_session(&self.internal.session)?;
        Ok(session.session_config().into())
    }

    pub fn set_session_config(&self, config: PySessionConfiguration) -> PyResult<()> {
        let session = strong_session(&self.internal.session)?;
        session
            .set_session_config(&config.into())
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }
}

/// High-level session classification presented to Python.
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub enum PySessionType {
    /// Point-to-point with a single, explicit destination name.
    #[pyo3(name = "P2P")]
    P2P = 0,
    /// Many-to-many distribution via a group channel_name.
    #[pyo3(name = "GROUP")]
    Group = 1,
}

impl From<session::SessionType> for PySessionType {
    fn from(value: session::SessionType) -> Self {
        match value {
            session::SessionType::PointToPoint => PySessionType::P2P,
            session::SessionType::Multicast => PySessionType::Group,
        }
    }
}

/// User-facing configuration for establishing and tuning sessions.
///
/// Each variant maps to a core `SessionConfig`.
/// Common fields (casual rundown):
/// * `timeout`: How long we wait for an ack before trying again.
/// * `max_retries`: Number of attempts to send a message. If we run out, an error is returned.
/// * `mls_enabled`: Turn on MLS for end‑to‑end crypto.
/// * `metadata`: One-shot string key/value tags sent at session start; the other side can read them for tracing, routing, auth, etc.
///
/// Variant-specific notes:
/// * `P2P`: P2P will target a specific peer for all messages.
/// * `Group`: Uses a named channel and distributes to multiple subscribers.
///
/// # Examples
///
/// ## Python: Create different session configs
/// ```python
/// from slim_bindings import PySessionConfiguration, PyName
///
/// # P2P session. Wait up to 2 seconds for an ack for each message, retry up to 5 times,
/// # enable MLS, and attach some metadata.
/// p2p_cfg = PySessionConfiguration.P2P(
///     peer_name=PyName("org", "namespace", "service"), # target peer
///     timeout=datetime.timedelta(seconds=2), # wait 2 seconds for an ack
///     max_retries=5, # retry up to 5 times
///     mls_enabled=True, # enable MLS
///     metadata={"trace_id": "1234abcd"} # arbitrary (string -> string) key/value pairs to send at session establishment
/// )
///
/// # Group session (channel-based)
/// channel = PyName("org", "namespace", "channel")
/// group_cfg = PySessionConfiguration.Group(
///     channel_name=channel, # group channel_name
///     max_retries=2, # retry up to 2 times
///     timeout=datetime.timedelta(seconds=2), # wait 2 seconds for an ack
///     mls_enabled=True, # enable MLS
///     metadata={"role": "publisher"} # arbitrary (string -> string) key/value pairs to send at session establishment
/// )
/// ```
///
/// ## Python: Using a config when creating a session
/// ```python
/// slim = await Slim.new(local_name, provider, verifier)
/// session = await slim.create_session(p2p_cfg)
/// print("Session ID:", session.id)
/// print("Type:", session.session_type)
/// print("Metadata:", session.metadata)
/// ```
///
/// ## Python: Updating configuration after creation
/// ```python
/// # Adjust retries & metadata dynamically
/// new_cfg = PySessionConfiguration.P2P(
///     peer_name=PyName("org", "namespace", "service"),
///     timeout=None,
///     max_retries=10,
///     mls_enabled=True,
///     metadata={"trace_id": "1234abcd", "phase": "retrying"}
/// )
/// session.set_session_config(new_cfg)
/// ```
///
/// ## Rust (internal conversion flow)
/// The enum transparently converts to and from `session::SessionConfig`:
/// ```rust
/// let core: session::SessionConfig = py_cfg.clone().into();
/// let roundtrip: PySessionConfiguration = core.into();
/// assert_eq!(py_cfg, roundtrip);
/// ```
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq, str)]
pub(crate) enum PySessionConfiguration {
    /// P2P configuration with a fixed destination (peer_name).
    #[pyo3(constructor = (peer_name, timeout=None, max_retries=None, mls_enabled=false, metadata=HashMap::new()))]
    P2P {
        peer_name: PyName,
        timeout: Option<std::time::Duration>,
        /// Optional maximum retry attempts.
        max_retries: Option<u32>,
        /// Enable (true) or disable (false) MLS features.
        mls_enabled: bool,
        /// Arbitrary metadata key/value pairs.
        metadata: HashMap<String, String>,
    },

    /// Group configuration: one-to-many distribution through a channel_name.
    #[pyo3(constructor = (channel_name, max_retries=0, timeout=std::time::Duration::from_millis(1000), mls_enabled=false, metadata=HashMap::new()))]
    Group {
        /// Group channel_name (channel) identifier.
        channel_name: PyName,
        /// Maximum retry attempts for setup or message send.
        max_retries: u32,
        /// Per-operation timeout.
        timeout: std::time::Duration,
        /// Enable (true) or disable (false) MLS features.
        mls_enabled: bool,
        /// Arbitrary metadata key/value pairs.
        metadata: HashMap<String, String>,
    },
}

// TODO(msardara): unify the configs as now they became identical
#[pymethods]
impl PySessionConfiguration {
    /// Return the name of the destination (peer or channel).
    #[getter]
    pub fn destination_name(&self) -> PyName {
        match self {
            PySessionConfiguration::P2P { peer_name, .. } => peer_name.clone(),
            PySessionConfiguration::Group { channel_name, .. } => channel_name.clone(),
        }
    }

    /// Return the metadata map (cloned).
    #[getter]
    pub fn metadata(&self) -> HashMap<String, String> {
        match self {
            PySessionConfiguration::P2P { metadata, .. } => metadata.clone(),
            PySessionConfiguration::Group { metadata, .. } => metadata.clone(),
        }
    }

    /// Return whether MLS is enabled.
    #[getter]
    pub fn mls_enabled(&self) -> bool {
        match self {
            PySessionConfiguration::P2P { mls_enabled, .. } => *mls_enabled,
            PySessionConfiguration::Group { mls_enabled, .. } => *mls_enabled,
        }
    }

    /// Return the timeout duration (if any).
    #[getter]
    pub fn timeout(&self) -> Option<std::time::Duration> {
        match self {
            PySessionConfiguration::P2P { timeout, .. } => *timeout,
            PySessionConfiguration::Group { timeout, .. } => Some(*timeout),
        }
    }

    /// Return the maximum number of retries (if any).
    #[getter]
    pub fn max_retries(&self) -> Option<u32> {
        match self {
            PySessionConfiguration::P2P { max_retries, .. } => *max_retries,
            PySessionConfiguration::Group { max_retries, .. } => Some(*max_retries),
        }
    }
}

impl Display for PySessionConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PySessionConfiguration::P2P {
                peer_name,
                timeout,
                max_retries,
                mls_enabled,
                metadata,
            } => write!(
                f,
                "P2P(peer_name={}, timeout={:?}, max_retries={:?}, mls_enabled={}, metadata={:?})",
                peer_name, timeout, max_retries, mls_enabled, metadata
            ),
            PySessionConfiguration::Group {
                channel_name,
                max_retries,
                timeout,
                mls_enabled,
                metadata,
            } => write!(
                f,
                "Group(channel_name={}, max_retries={}, timeout={:?}, mls_enabled={}, metadata={:?})",
                channel_name, max_retries, timeout, mls_enabled, metadata
            ),
        }
    }
}

impl From<session::SessionConfig> for PySessionConfiguration {
    fn from(session_config: session::SessionConfig) -> Self {
        match session_config {
            session::SessionConfig::PointToPoint(config) => PySessionConfiguration::P2P {
                peer_name: config.peer_name.expect("peer name not set").into(),
                timeout: config.timeout,
                max_retries: config.max_retries,
                mls_enabled: config.mls_enabled,
                metadata: config.metadata,
            },
            session::SessionConfig::Multicast(config) => PySessionConfiguration::Group {
                channel_name: config.channel_name.into(),
                max_retries: config.max_retries,
                timeout: config.timeout,
                mls_enabled: config.mls_enabled,
                metadata: config.metadata,
            },
        }
    }
}

impl From<PySessionConfiguration> for session::SessionConfig {
    fn from(value: PySessionConfiguration) -> Self {
        match value {
            PySessionConfiguration::P2P {
                peer_name,
                timeout,
                max_retries,
                mls_enabled,
                metadata,
            } => session::SessionConfig::PointToPoint(PointToPointConfiguration::new(
                timeout,
                max_retries,
                mls_enabled,
                Some(peer_name.into()),
                metadata,
            )),
            PySessionConfiguration::Group {
                channel_name,
                max_retries,
                timeout,
                mls_enabled,
                metadata,
            } => session::SessionConfig::Multicast(MulticastConfiguration::new(
                channel_name.into(),
                Some(max_retries),
                Some(timeout),
                mls_enabled,
                metadata,
            )),
        }
    }
}
