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

pub(crate) struct PySessionCtxInternal {
    pub(crate) session: Weak<Session<IdentityProvider, IdentityVerifier>>,
    pub(crate) rx: RwLock<AppChannelReceiver>,
}

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PySessionContext {
    pub(crate) internal: Arc<PySessionCtxInternal>,
}

impl From<SessionContext<IdentityProvider, IdentityVerifier>> for PySessionContext {
    fn from(ctx: SessionContext<IdentityProvider, IdentityVerifier>) -> Self {
        // split context into parts
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

    pub fn set_session_config(&self, config: PySessionConfiguration) -> PyResult<()> {
        let session = strong_session(&self.internal.session)?;
        session
            .set_session_config(&config.into())
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }
}

/// session type
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub enum PySessionType {
    #[pyo3(name = "ANYCAST")]
    Anycast = 0,
    #[pyo3(name = "UNICAST")]
    Unicast = 1,
    #[pyo3(name = "MULTICAST")]
    Multicast = 2,
}

#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
pub(crate) enum PySessionConfiguration {
    #[pyo3(constructor = (timeout=None, max_retries=None, mls_enabled=false, metadata=HashMap::new()))]
    Anycast {
        timeout: Option<std::time::Duration>,
        max_retries: Option<u32>,
        mls_enabled: bool,
        metadata: HashMap<String, String>,
    },

    #[pyo3(constructor = (unicast_name, timeout=None, max_retries=None, mls_enabled=false, metadata=HashMap::new()))]
    Unicast {
        unicast_name: PyName,
        timeout: Option<std::time::Duration>,
        max_retries: Option<u32>,
        mls_enabled: bool,
        metadata: HashMap<String, String>,
    },

    #[pyo3(constructor = (topic, max_retries=0, timeout=std::time::Duration::from_millis(1000), mls_enabled=false, metadata=HashMap::new()))]
    Multicast {
        topic: PyName,
        max_retries: u32,
        timeout: std::time::Duration,
        mls_enabled: bool,
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
                        metadata: config.metadata,
                    }
                }
            }
            session::SessionConfig::Multicast(config) => PySessionConfiguration::Multicast {
                topic: config.channel_name.into(),
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
                metadata,
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
                topic,
                max_retries,
                timeout,
                mls_enabled,
                metadata,
            } => session::SessionConfig::Multicast(MulticastConfiguration::new(
                topic.into(),
                Some(max_retries),
                Some(timeout),
                mls_enabled,
                metadata,
            )),
        }
    }
}
