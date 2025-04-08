// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyclass_enum;
use pyo3_stub_gen::derive::gen_stub_pymethods;

use crate::utils::PyAgentType;
use agp_service::session;

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

/// fire and forget session config
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone, Default)]
pub(crate) struct PyFireAndForgetConfiguration {
    pub fire_and_forget_configuration: agp_service::FireAndForgetConfiguration,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyFireAndForgetConfiguration {
    #[new]
    pub fn new() -> Self {
        PyFireAndForgetConfiguration {
            fire_and_forget_configuration: agp_service::FireAndForgetConfiguration {},
        }
    }
}

/// request response session config
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone, Default)]
pub(crate) struct PyRequestResponseConfiguration {
    pub request_response_configuration: agp_service::RequestResponseConfiguration,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyRequestResponseConfiguration {
    #[new]
    #[pyo3(signature = (max_retries=0, timeout=1000))]
    pub fn new(max_retries: u32, timeout: u32) -> Self {
        PyRequestResponseConfiguration {
            request_response_configuration: agp_service::RequestResponseConfiguration {
                max_retries,
                timeout: std::time::Duration::from_millis(timeout as u64),
            },
        }
    }

    #[getter]
    pub fn max_retries(&self) -> u32 {
        self.request_response_configuration.max_retries
    }

    #[getter]
    pub fn timeout(&self) -> u32 {
        self.request_response_configuration.timeout.as_millis() as u32
    }

    #[setter]
    pub fn set_max_retries(&mut self, max_retries: u32) {
        self.request_response_configuration.max_retries = max_retries;
    }

    #[setter]
    pub fn set_timeout(&mut self, timeout: u32) {
        self.request_response_configuration.timeout =
            std::time::Duration::from_millis(timeout as u64);
    }
}

/// streaming session config
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PyStreamingConfiguration {
    pub streaming_configuration: agp_service::StreamingConfiguration,
    topic: Option<PyAgentType>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyStreamingConfiguration {
    #[new]
    #[pyo3(signature = (direction, topic, max_retries=None, timeout=None))]
    pub fn new(
        direction: PySessionDirection,
        topic: Option<PyAgentType>,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        Self {
            streaming_configuration: agp_service::StreamingConfiguration::new(
                direction.into(),
                topic.clone().map(|t| t.into()),
                max_retries,
                timeout,
            ),
            topic,
        }
    }

    #[getter]
    pub fn direction(&self) -> PySessionDirection {
        self.streaming_configuration.direction.clone().into()
    }

    #[getter]
    pub fn topic(&self) -> Option<PyAgentType> {
        self.topic.clone()
    }

    #[getter]
    pub fn max_retries(&self) -> u32 {
        self.streaming_configuration.max_retries
    }
}
