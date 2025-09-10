// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyclass_enum;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use slim_datapath::messages::Name;

use crate::utils::PyName;
use slim_service::MulticastConfiguration;
use slim_service::PointToPointConfiguration;
use slim_service::session;
pub use slim_service::session::SESSION_UNSPECIFIED;

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PySessionInfo {
    pub(crate) session_info: session::Info,
}

impl From<session::Info> for PySessionInfo {
    fn from(session_info: session::Info) -> Self {
        PySessionInfo { session_info }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PySessionInfo {
    #[new]
    fn new(session_id: u32) -> Self {
        PySessionInfo {
            session_info: session::Info::new(session_id),
        }
    }

    #[getter]
    fn id(&self) -> u32 {
        self.session_info.id
    }

    #[getter]
    fn source_name(&self) -> PyName {
        let name = match &self.session_info.message_source {
            Some(n) => n.clone(),
            None => Name::from_strings(["", "", ""]),
        };
        PyName::from(name)
    }

    #[getter]
    pub fn destination_name(&self) -> PyName {
        let name = match &self.session_info.message_destination {
            Some(n) => n.clone(),
            None => Name::from_strings(["", "", ""]),
        };
        PyName::from(name)
    }

    #[getter]
    pub fn payload_type(&self) -> String {
        match &self.session_info.payload_type {
            Some(t) => t.clone(),
            None => "".to_string(),
        }
    }

    #[getter]
    pub fn metadata(&self) -> HashMap<String, String> {
        self.session_info.metadata.clone()
    }
}

/// session type
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub(crate) enum PySessionType {
    #[pyo3(name = "FIRE_AND_FORGET")]
    PointToPoint = session::SessionType::PointToPoint as isize,
    #[pyo3(name = "MULTICAST")]
    Multicast = session::SessionType::Multicast as isize,
}

impl From<PySessionType> for session::SessionType {
    fn from(value: PySessionType) -> Self {
        match value {
            PySessionType::PointToPoint => session::SessionType::PointToPoint,
            PySessionType::Multicast => session::SessionType::Multicast,
        }
    }
}

#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
pub(crate) enum PySessionConfiguration {
    #[pyo3(constructor = (timeout=None, max_retries=None, sticky=false, mls_enabled=false))]
    PointToPoint {
        timeout: Option<std::time::Duration>,
        max_retries: Option<u32>,
        sticky: bool,
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
            session::SessionConfig::PointToPoint(config) => PySessionConfiguration::PointToPoint {
                timeout: config.timeout,
                max_retries: config.max_retries,
                sticky: config.sticky,
                mls_enabled: config.mls_enabled,
            },
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
            PySessionConfiguration::PointToPoint {
                timeout,
                max_retries,
                sticky,
                mls_enabled,
            } => session::SessionConfig::PointToPoint(PointToPointConfiguration::new(
                timeout,
                max_retries,
                sticky,
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
