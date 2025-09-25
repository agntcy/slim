// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::exceptions::PyException;
use std::collections::HashMap;
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
/// A thin, clonable handle around the underlying Rust session state. All
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
///   (ANYCAST, UNICAST, MULTICAST), inferred from internal kind + destination.
/// - src -> PyName: Fully qualified source identity that originated / owns
///   the session.
/// - dst -> Optional[PyName]: Destination name when applicable:
///     * PyName of the peer for UNICAST
///     * None for ANYCAST (no fixed peer)
///     * PyName of the channel for MULTICAST
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

        let session_type = session.kind();
        let dst = session.dst();

        match (session_type, dst) {
            (session::SessionType::PointToPoint, Some(_)) => Ok(PySessionType::Unicast),
            (session::SessionType::PointToPoint, None) => Ok(PySessionType::Anycast),
            (session::SessionType::Multicast, _) => Ok(PySessionType::Multicast),
        }
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

    #[getter]
    pub fn destination_name(&self) -> PyResult<Option<PyName>> {
        let session = strong_session(&self.internal.session)?;
        let session_config = session.session_config();

        let name_opt = match session_config {
            session::SessionConfig::PointToPoint(cfg) => {
                cfg.unicast_name.as_ref().map(|n| n.clone().into())
            } // None if Anycast
            session::SessionConfig::Multicast(cfg) => Some(cfg.channel_name.clone().into()),
        };

        Ok(name_opt)
    }

    /// Replace the underlying session configuration with a new one.
    ///
    /// Safety/Consistency:
    /// The underlying service validates and applies changes atomically.
    /// Errors (e.g. invalid transitions) are surfaced as Python exceptions.
    pub fn set_session_config(&self, config: PySessionConfiguration) -> PyResult<()> {
        let session = strong_session(&self.internal.session)?;
        session
            .set_session_config(&config.into())
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }
}

/// High-level session classification presented to Python.
///
/// Variants map onto core `SessionType` plus additional inference
/// (e.g. presence of a concrete destination for UNICAST).
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub enum PySessionType {
    /// Point-to-point without a fixed destination (best-available / load-balanced).
    #[pyo3(name = "ANYCAST")]
    Anycast = 0,
    /// Point-to-point with a single, explicit destination name.
    #[pyo3(name = "UNICAST")]
    Unicast = 1,
    /// One-to-many distribution via a multicast channel_name/channel.
    #[pyo3(name = "MULTICAST")]
    Multicast = 2,
}

/// User-facing configuration for establishing and tuning sessions.
///
/// Each variant corresponds to an underlying core `SessionConfig`.
/// Common fields:
/// * `timeout`: How long to wait for operations (creation / messaging) before failing.
/// * `max_retries`: Optional retry count for establishment or delivery.
/// * `mls_enabled`: Whether to negotiate/use MLS secure group messaging.
/// * `metadata`: Free-form string map propagated with session context.
///
/// Variant-specific notes:
/// * `Anycast` / `Unicast`: Point-to-point; anycast will pick any available peer
///                          for each message sent, while unicast targets a specific
///                          peer for all messages.
/// * `Multicast`: Uses a named channel and distributes to multiple subscribers.
///
/// # Examples
///
/// ## Python: Create different session configs
/// ```python
/// from slim_bindings import PySessionConfiguration, PyName
///
/// # Anycast session (no fixed destination; service picks an available peer)
/// # MLS is not available with Anycast sessions, and session metadata is not supported,
/// # as there is no session establishment phase, only per-message routing.
/// anycast_cfg = PySessionConfiguration.Anycast
///     timeout=datetime.timedelta(seconds=2), # try to send a message within 2 seconds
///     max_retries=5, # retry up to 5 times
/// )
///
/// # Unicast session. Try to send a message within 2 seconds, retry up to 5 times,
/// # enable MLS, and attach some metadata.
/// unicast_cfg = PySessionConfiguration.Unicast(
///     timeout=datetime.timedelta(seconds=2), # try to send a message within 2 seconds
///     max_retries=5, # retry up to 5 times
///     mls_enabled=True, # enable MLS
///     metadata={"trace_id": "1234abcd"} # arbitrary key/value pairs to send at session establishment
/// )
///
/// # Multicast session (channel-based)
/// channel = PyName("org", "namespace", "channel")
/// multicast_cfg = PySessionConfiguration.Multicast(
///     channel, # multicast channel_name
///     max_retries=2, # retry up to 2 times
///     timeout=datetime.timedelta(seconds=2), # try to send a message within 2 seconds
///     mls_enabled=True, # enable MLS
///     metadata={"role": "publisher"} # arbitrary key/value pairs to send at session establishment
/// )
/// ```
///
/// ## Python: Using a config when creating a session
/// ```python
/// slim = await Slim.new(local_name, provider, verifier)
/// session = await slim.create_session(unicast_cfg)
/// print("Session ID:", session.id)
/// print("Type:", session.session_type)
/// print("Metadata:", session.metadata)
/// ```
///
/// ## Python: Updating configuration after creation
/// ```python
/// # Adjust retries & metadata dynamically
/// new_cfg = PySessionConfiguration.Unicast(
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
#[pyclass(eq)]
pub(crate) enum PySessionConfiguration {
    /// Point-to-point configuration without a fixed destination; selection of
    /// the actual peer is deferred/implicit (load-balancing behavior).
    #[pyo3(constructor = (timeout=None, max_retries=None, mls_enabled=false))]
    Anycast {
        /// Optional overall timeout for operations.
        timeout: Option<std::time::Duration>,
        /// Optional maximum retry attempts.
        max_retries: Option<u32>,
        /// Enable (true) or disable (false) MLS features.
        mls_enabled: bool,
    },

    #[pyo3(constructor = (unicast_name, timeout=None, max_retries=None, mls_enabled=false, metadata=HashMap::new()))]
    Unicast {
        unicast_name: PyName,
        timeout: Option<std::time::Duration>,
        /// Optional maximum retry attempts.
        max_retries: Option<u32>,
        /// Enable (true) or disable (false) MLS features.
        mls_enabled: bool,
        /// Arbitrary metadata key/value pairs.
        metadata: HashMap<String, String>,
    },

    /// Multicast configuration: one-to-many distribution through a channel_name.
    #[pyo3(constructor = (channel_name, max_retries=0, timeout=std::time::Duration::from_millis(1000), mls_enabled=false, metadata=HashMap::new()))]
    Multicast {
        /// Multicast channel_name (channel) identifier.
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

impl From<session::SessionConfig> for PySessionConfiguration {
    fn from(session_config: session::SessionConfig) -> Self {
        match session_config {
            session::SessionConfig::PointToPoint(config) => {
                if config.unicast_name.is_some() {
                    PySessionConfiguration::Unicast {
                        unicast_name: config.unicast_name.unwrap().into(),
                        timeout: config.timeout,
                        max_retries: config.max_retries,
                        mls_enabled: config.mls_enabled,
                        metadata: config.metadata,
                    }
                } else {
                    PySessionConfiguration::Anycast {
                        timeout: config.timeout,
                        max_retries: config.max_retries,
                        mls_enabled: config.mls_enabled,
                    }
                }
            }
            session::SessionConfig::Multicast(config) => PySessionConfiguration::Multicast {
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
            PySessionConfiguration::Anycast {
                timeout,
                max_retries,
                mls_enabled,
            } => session::SessionConfig::PointToPoint(PointToPointConfiguration::new(
                timeout,
                max_retries,
                mls_enabled,
                None,
                metadata,
            )),
            PySessionConfiguration::Unicast {
                unicast_name,
                timeout,
                max_retries,
                mls_enabled,
                metadata,
            } => session::SessionConfig::PointToPoint(PointToPointConfiguration::new(
                timeout,
                max_retries,
                mls_enabled,
                Some(unicast_name.into()),
                metadata,
            )),
            PySessionConfiguration::Multicast {
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
