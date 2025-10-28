// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::exceptions::PyException;
use slim_datapath::api::ProtoSessionType;
use slim_session::session_controller::SessionController;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, Weak};

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyclass_enum;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use slim_session::{SessionError, session_controller::SessionConfig};

use crate::pyidentity::{IdentityProvider, IdentityVerifier};
use crate::pymessage::PyMessageContext;
use crate::utils::PyName;
use slim_datapath::messages::Name;
use slim_service::{BindingsAdapter, BindingsSessionContext, MessageContext};
pub use slim_session::SESSION_UNSPECIFIED;

use slim_session::context::SessionContext;

/// Internal shared session context state.
///
/// Holds a `BindingsSessionContext` which provides:
/// * Session-specific operations (publish, invite, remove, get_message)
/// * A weak reference to the underlying `Session` (so that Python
///   references do not keep a closed session alive)
/// * A receiver (`rx`) for application/channel messages which is
///   protected by an async `RwLock` to allow concurrent access patterns
///
/// This struct is not exposed directly to Python; it is wrapped by
/// `PySessionContext`.
pub(crate) struct PySessionCtxInternal {
    pub(crate) bindings_ctx: BindingsSessionContext<IdentityProvider, IdentityVerifier>,
}

/// Python-exposed session context wrapper.
///
/// A thin, cloneable handle around the underlying Rust session state that provides
/// both session metadata access and session-specific operations. All getters perform
/// a safe upgrade of the weak internal session reference, returning a Python exception
/// if the session has already been closed.
///
/// Properties (getters exposed to Python):
/// - id -> int: Unique numeric identifier of the session. Raises a Python
///   exception if the session has been closed.
/// - metadata -> dict[str,str]: Arbitrary key/value metadata copied from the
///   current SessionConfig. A cloned map is returned so Python can mutate
///   without racing the underlying config.
/// - session_type -> PySessionType: High-level transport classification
///   (PointToPoint, Group), inferred from internal kind + destination.
/// - src -> PyName: Fully qualified source identity that originated / owns
///   the session.
/// - dst -> PyName: Destination name:
///     * PyName of the peer for PointToPoint
///     * PyName of the channel for Group
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
        // Convert to BindingsSessionContext
        let bindings_ctx = BindingsSessionContext::from(ctx);

        PySessionContext {
            internal: Arc::new(PySessionCtxInternal { bindings_ctx }),
        }
    }
}

// Internal helper to obtain a strong session reference or raise a Python exception
fn strong_session(
    weak: &Weak<SessionController<IdentityProvider, IdentityVerifier>>,
) -> PyResult<Arc<SessionController<IdentityProvider, IdentityVerifier>>> {
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
        let id = strong_session(&self.internal.bindings_ctx.session)?.id();

        Ok(id)
    }

    #[getter]
    pub fn metadata(&self) -> PyResult<HashMap<String, String>> {
        let session = self
            .internal
            .bindings_ctx
            .session
            .upgrade()
            .ok_or_else(|| {
                PyErr::new::<PyException, _>(
                    SessionError::SessionClosed("session already closed".to_string()).to_string(),
                )
            })?;

        Ok(session.metadata())
    }

    #[getter]
    pub fn session_type(&self) -> PyResult<PySessionType> {
        let session = strong_session(&self.internal.bindings_ctx.session)?;
        Ok(session.session_type().into())
    }

    #[getter]
    pub fn src(&self) -> PyResult<PyName> {
        let session = strong_session(&self.internal.bindings_ctx.session)?;

        Ok(session.source().clone().into())
    }

    #[getter]
    pub fn dst(&self) -> PyResult<Option<PyName>> {
        let session = strong_session(&self.internal.bindings_ctx.session)?;

        Ok(Some(session.dst().clone().into()))
    }

    #[getter]
    pub fn session_config(&self) -> PyResult<PySessionConfiguration> {
        let session = strong_session(&self.internal.bindings_ctx.session)?;
        Ok(session.session_config().into())
    }
}

/// Session-specific operations
impl PySessionContext {
    /// Get a message from this session
    pub(crate) async fn get_message(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> PyResult<(PyMessageContext, Vec<u8>)> {
        let (ctx, payload) = self
            .internal
            .bindings_ctx
            .get_session_message(timeout)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok((ctx.into(), payload))
    }

    /// Publish a message through this session
    pub(crate) async fn publish(
        &self,
        fanout: u32,
        blob: Vec<u8>,
        message_ctx: Option<PyMessageContext>,
        name: Option<PyName>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> PyResult<()> {
        let session = self
            .internal
            .bindings_ctx
            .session
            .upgrade()
            .ok_or_else(|| PyErr::new::<PyException, _>("session closed"))?;

        let (target_name, conn_out) = match &name {
            Some(name) => (name, None),
            None => match &message_ctx {
                Some(ctx) => (&ctx.source_name, Some(ctx.input_connection)),
                None => (&PyName::from(session.dst().clone()), None),
            },
        };

        let target_name = Name::from(target_name);

        self.internal
            .bindings_ctx
            .publish(&target_name, fanout, blob, conn_out, payload_type, metadata)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }

    /// Publish a message as a reply to a received message
    pub(crate) async fn publish_to(
        &self,
        message_ctx: PyMessageContext,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> PyResult<()> {
        let ctx: MessageContext = message_ctx.into();

        self.internal
            .bindings_ctx
            .publish_to(&ctx, blob, payload_type, metadata)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }

    /// Invite a participant to this session (multicast only)
    pub(crate) async fn invite(&self, name: PyName) -> PyResult<()> {
        self.internal
            .bindings_ctx
            .invite(&name.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }

    /// Remove a participant from this session (multicast only)
    pub(crate) async fn remove(&self, name: PyName) -> PyResult<()> {
        self.internal
            .bindings_ctx
            .remove(&name.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }

    /// Delete this session
    pub(crate) async fn delete(
        &self,
        adapter: &BindingsAdapter<IdentityProvider, IdentityVerifier>,
    ) -> PyResult<()> {
        let session = self
            .internal
            .bindings_ctx
            .session
            .upgrade()
            .ok_or_else(|| PyErr::new::<PyException, _>("session closed"))?;

        adapter
            .delete_session(&session)
            .await
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
    #[pyo3(name = "PointToPoint")]
    PointToPoint = 0,
    /// Many-to-many distribution via a group channel_name.
    #[pyo3(name = "Group")]
    Group = 1,
}

impl From<ProtoSessionType> for PySessionType {
    fn from(value: ProtoSessionType) -> Self {
        match value {
            ProtoSessionType::PointToPoint => PySessionType::PointToPoint,
            ProtoSessionType::Multicast => PySessionType::Group,
            ProtoSessionType::Unspecified => panic!("unexpected session type"),
        }
    }
}

/// User-facing configuration for establishing and tuning sessions.
///
/// Each variant maps to a core `SessionConfig` and defines the behavior of session-level
/// operations like message publishing, participant management, and message reception.
///
/// Common fields:
/// * `timeout`: How long we wait for an ack before trying again.
/// * `max_retries`: Number of attempts to send a message. If we run out, an error is returned.
/// * `mls_enabled`: Turn on MLS for end‑to‑end crypto.
/// * `metadata`: One-shot string key/value tags sent at session start; the other side can read them for tracing, routing, auth, etc.
///
/// Variant-specific notes:
/// * `PointToPoint`: Direct communication with a specific peer. Session operations target the peer directly.
/// * `Group`: Channel-based multicast communication. Session operations affect the entire group.
///
/// # Examples
///
/// ## Python: Create different session configs
/// ```python
/// from slim_bindings import PySessionConfiguration, PyName
///
/// # PointToPoint session - direct peer communication
/// p2p_cfg = PySessionConfiguration.PointToPoint(
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
/// new_cfg = PySessionConfiguration.PointToPoint(
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
/// The enum transparently converts to and from `SessionConfig`:
/// ```
/// // Example conversion (pseudo-code):
/// // let core: SessionConfig = py_cfg.clone().into();
/// // let roundtrip: PySessionConfiguration = core.into();
/// // assert_eq!(py_cfg, roundtrip);
/// ```
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub struct PySessionConfiguration {
    /// session type
    session_type: ProtoSessionType,
    /// Optional maximum retry attempts.
    max_retries: Option<u32>,
    /// interval between attempts
    timeout: Option<std::time::Duration>,
    /// Enable (true) or disable (false) MLS features.
    mls_enabled: bool,
    /// True is this endpoint is the initiator of the session
    initiator: bool,
    /// Arbitrary metadata key/value pairs.
    metadata: HashMap<String, String>,
}

// TODO(msardara): unify the configs as now they became identical
#[pymethods]
impl PySessionConfiguration {
    /// Return the metadata map (cloned).
    #[getter]
    pub fn metadata(&self) -> HashMap<String, String> {
        self.metadata.clone()
    }

    /// Return whether MLS is enabled.
    #[getter]
    pub fn mls_enabled(&self) -> bool {
        self.mls_enabled
    }

    /// Return the timeout duration (if any).
    #[getter]
    pub fn timeout(&self) -> Option<std::time::Duration> {
        self.timeout
    }

    /// Return the maximum number of retries (if any).
    #[getter]
    pub fn max_retries(&self) -> Option<u32> {
        self.max_retries
    }
}

impl Display for PySessionConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SessionConfig(timeout={:?}, max_retries={:?}, mls_enabled={}, metadata={:?})",
            self.timeout, self.max_retries, self.mls_enabled, self.metadata
        )
    }
}

impl From<SessionConfig> for PySessionConfiguration {
    fn from(session_config: SessionConfig) -> Self {
        PySessionConfiguration {
            session_type: session_config.session_type,
            timeout: session_config.duration,
            max_retries: session_config.max_retries,
            mls_enabled: session_config.mls_enabled,
            initiator: session_config.initiator,
            metadata: session_config.metadata,
        }
    }
}

impl From<&PySessionConfiguration> for SessionConfig {
    fn from(value: &PySessionConfiguration) -> Self {
        SessionConfig {
            session_type: value.session_type,
            duration: value.timeout,
            max_retries: value.max_retries,
            mls_enabled: value.mls_enabled,
            initiator: value.initiator,
            metadata: value.metadata.clone(),
        }
    }
}

// ============================================================================
// Python binding functions for session operations
// ============================================================================

/// Publish a message through the specified session.
#[allow(clippy::too_many_arguments)]
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (session_context, fanout, blob, message_ctx=None, name=None, payload_type=None, metadata=None))]
pub fn publish(
    py: Python,
    session_context: PySessionContext,
    fanout: u32,
    blob: Vec<u8>,
    message_ctx: Option<PyMessageContext>,
    name: Option<PyName>,
    payload_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        session_context
            .publish(fanout, blob, message_ctx, name, payload_type, metadata)
            .await
    })
}

/// Publish a message as a reply to a received message through the specified session.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (session_context, message_ctx, blob, payload_type=None, metadata=None))]
pub fn publish_to(
    py: Python,
    session_context: PySessionContext,
    message_ctx: PyMessageContext,
    blob: Vec<u8>,
    payload_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py_with_locals(
        py,
        pyo3_async_runtimes::tokio::get_current_locals(py)?,
        async move {
            session_context
                .publish_to(message_ctx, blob, payload_type, metadata)
                .await
        },
    )
}

/// Invite a participant to the specified session (group only).
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (session_context, name))]
pub fn invite(
    py: Python,
    session_context: PySessionContext,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(
        py,
        async move { session_context.invite(name).await },
    )
}

/// Remove a participant from the specified session (group only).
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (session_context, name))]
pub fn remove(
    py: Python,
    session_context: PySessionContext,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(
        py,
        async move { session_context.remove(name).await },
    )
}

/// Get a message from the specified session.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (session_context, timeout=None))]
pub fn get_message(
    py: Python,
    session_context: PySessionContext,
    timeout: Option<std::time::Duration>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py_with_locals(
        py,
        pyo3_async_runtimes::tokio::get_current_locals(py)?,
        async move { session_context.get_message(timeout).await },
    )
}
