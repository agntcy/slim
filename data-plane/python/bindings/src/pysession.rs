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
    pub(crate) metadata: HashMap<String, String>,
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
        let (session, rx, metadata) = ctx.into_parts();
        let rx = RwLock::new(rx);

        PySessionContext {
            internal: Arc::new(PySessionCtxInternal {
                session,
                rx,
                metadata: metadata.unwrap_or(HashMap::new()),
            }),
        }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PySessionContext {
    #[getter]
    pub fn id(&self) -> PyResult<u32> {
        let id = self
            .internal
            .session
            .upgrade()
            .ok_or_else(|| SessionError::SessionClosed("session already closed".to_string()))
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?
            .id();

        Ok(id)
    }

    #[getter]
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.internal.metadata
    }

    pub fn set_session_config(&self, config: PySessionConfiguration) -> PyResult<()> {
        let session = self.internal.session.upgrade().ok_or_else(|| {
            PyErr::new::<PyException, _>(
                SessionError::SessionClosed("session already closed".to_string()).to_string(),
            )
        })?;
        session
            .set_session_config(&config.into())
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;
        Ok(())
    }

    pub fn get_session_config(&self) -> PyResult<PySessionConfiguration> {
        let session = self.internal.session.upgrade().ok_or_else(|| {
            PyErr::new::<PyException, _>(
                SessionError::SessionClosed("session already closed".to_string()).to_string(),
            )
        })?;
        Ok(session.session_config().into())
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
    #[pyo3(constructor = (timeout=None, max_retries=None, mls_enabled=false))]
    Anycast {
        timeout: Option<std::time::Duration>,
        max_retries: Option<u32>,
        mls_enabled: bool,
    },

    #[pyo3(constructor = (timeout=None, max_retries=None, mls_enabled=false))]
    Unicast {
        timeout: Option<std::time::Duration>,
        max_retries: Option<u32>,
        mls_enabled: bool,
    },

    #[pyo3(constructor = (topic, moderator=false, max_retries=0, timeout=std::time::Duration::from_millis(1000), mls_enabled=false))]
    Multicast {
        topic: PyName,
        moderator: bool,
        max_retries: u32,
        timeout: std::time::Duration,
        mls_enabled: bool,
    },
}

impl From<session::SessionConfig> for PySessionConfiguration {
    fn from(session_config: session::SessionConfig) -> Self {
        match session_config {
            session::SessionConfig::PointToPoint(config) => {
                if config.unicast {
                    PySessionConfiguration::Unicast {
                        timeout: config.timeout,
                        max_retries: config.max_retries,
                        mls_enabled: config.mls_enabled,
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
                topic: config.channel_name.into(),
                moderator: config.moderator,
                max_retries: config.max_retries,
                timeout: config.timeout,
                mls_enabled: config.mls_enabled,
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
                false,
                mls_enabled,
            )),
            PySessionConfiguration::Unicast {
                timeout,
                max_retries,
                mls_enabled,
            } => session::SessionConfig::PointToPoint(PointToPointConfiguration::new(
                timeout,
                max_retries,
                true,
                mls_enabled,
            )),
            PySessionConfiguration::Multicast {
                topic,
                moderator,
                max_retries,
                timeout,
                mls_enabled,
            } => session::SessionConfig::Multicast(MulticastConfiguration::new(
                topic.into(),
                moderator,
                Some(max_retries),
                Some(timeout),
                mls_enabled,
            )),
        }
    }
}
