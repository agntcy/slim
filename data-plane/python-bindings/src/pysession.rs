// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyclass_enum;
use pyo3_stub_gen::derive::gen_stub_pymethods;

use crate::utils::PyAgentType;
use agp_service::FireAndForgetConfiguration;
use agp_service::RequestResponseConfiguration;
use agp_service::StreamingConfiguration;
use agp_service::session;
pub use agp_service::session::SESSION_UNSPECIFIED;

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
}

/// session direction
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub(crate) enum PySessionDirection {
    #[pyo3(name = "SENDER")]
    Sender = session::SessionDirection::Sender as isize,
    #[pyo3(name = "RECEIVER")]
    Receiver = session::SessionDirection::Receiver as isize,
    #[pyo3(name = "BIDIRECTIONAL")]
    Bidirectional = session::SessionDirection::Bidirectional as isize,
}

impl Into<session::SessionDirection> for PySessionDirection {
    fn into(self) -> session::SessionDirection {
        match self {
            PySessionDirection::Sender => session::SessionDirection::Sender,
            PySessionDirection::Receiver => session::SessionDirection::Receiver,
            PySessionDirection::Bidirectional => session::SessionDirection::Bidirectional,
        }
    }
}

impl From<session::SessionDirection> for PySessionDirection {
    fn from(session_direction: session::SessionDirection) -> Self {
        match session_direction {
            session::SessionDirection::Sender => PySessionDirection::Sender,
            session::SessionDirection::Receiver => PySessionDirection::Receiver,
            session::SessionDirection::Bidirectional => PySessionDirection::Bidirectional,
        }
    }
}

/// session type
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
#[derive(PartialEq, Clone)]
pub(crate) enum PySessionType {
    #[pyo3(name = "FIRE_AND_FORGET")]
    FireAndForget = session::SessionType::FireAndForget as isize,
    #[pyo3(name = "REQUEST_RESPONSE")]
    RequestResponse = session::SessionType::RequestResponse as isize,
    #[pyo3(name = "STREAMING")]
    Streaming = session::SessionType::Streaming as isize,
}

impl Into<session::SessionType> for PySessionType {
    fn into(self) -> session::SessionType {
        match self {
            PySessionType::FireAndForget => session::SessionType::FireAndForget,
            PySessionType::RequestResponse => session::SessionType::RequestResponse,
            PySessionType::Streaming => session::SessionType::Streaming,
        }
    }
}

/// request response session config
#[gen_stub_pyclass]
#[pyclass(eq)]
#[derive(Clone, Default, PartialEq)]
pub(crate) struct PyRequestResponseConfiguration {
    pub request_response_configuration: agp_service::RequestResponseConfiguration,
}

impl Into<agp_service::RequestResponseConfiguration> for PyRequestResponseConfiguration {
    fn into(self) -> agp_service::RequestResponseConfiguration {
        self.request_response_configuration
    }
}

impl From<agp_service::RequestResponseConfiguration> for PyRequestResponseConfiguration {
    fn from(request_response_configuration: agp_service::RequestResponseConfiguration) -> Self {
        PyRequestResponseConfiguration {
            request_response_configuration,
        }
    }
}

#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
pub(crate) enum PySessionConfiguration {
    FireAndForget {},
    #[pyo3(constructor = (timeout=std::time::Duration::from_millis(1000)))]
    RequestResponse {
        timeout: std::time::Duration,
    },
    #[pyo3(constructor = (session_direction, topic=None, max_retries=0, timeout=std::time::Duration::from_millis(1000)))]
    Streaming {
        session_direction: PySessionDirection,
        topic: Option<PyAgentType>,
        max_retries: u32,
        timeout: std::time::Duration,
    },
}

impl From<session::SessionConfig> for PySessionConfiguration {
    fn from(session_config: session::SessionConfig) -> Self {
        match session_config {
            session::SessionConfig::FireAndForget(_) => PySessionConfiguration::FireAndForget {},
            session::SessionConfig::RequestResponse(config) => {
                PySessionConfiguration::RequestResponse {
                    timeout: config.timeout,
                }
            }
            session::SessionConfig::Streaming(config) => PySessionConfiguration::Streaming {
                session_direction: config.direction.into(),
                topic: None,
                max_retries: config.max_retries,
                timeout: config.timeout,
            },
        }
    }
}

impl Into<session::SessionConfig> for PySessionConfiguration {
    fn into(self) -> session::SessionConfig {
        match self {
            PySessionConfiguration::FireAndForget {} => {
                session::SessionConfig::FireAndForget(FireAndForgetConfiguration {})
            }
            PySessionConfiguration::RequestResponse { timeout } => {
                session::SessionConfig::RequestResponse(RequestResponseConfiguration { timeout })
            }
            PySessionConfiguration::Streaming {
                session_direction,
                topic,
                max_retries,
                timeout,
            } => session::SessionConfig::Streaming(StreamingConfiguration::new(
                session_direction.into(),
                topic.map(|topic| topic.into()),
                Some(max_retries),
                Some(timeout),
            )),
        }
    }
}
