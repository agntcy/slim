// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use serde_pyobject::from_pyobject;
use slim_tracing::TracingConfiguration;
use tokio::sync::OnceCell;

use slim_datapath::messages::encoder::AgentType;

/// agent class
#[gen_stub_pyclass]
#[pyclass(eq)]
#[derive(Clone, PartialEq)]
pub struct PyAgentType {
    #[pyo3(get, set)]
    pub organization: String,

    #[pyo3(get, set)]
    pub namespace: String,

    #[pyo3(get, set)]
    pub agent_type: String,
}

impl From<PyAgentType> for AgentType {
    fn from(value: PyAgentType) -> AgentType {
        AgentType::from_strings(&value.organization, &value.namespace, &value.agent_type)
    }
}

impl From<&PyAgentType> for AgentType {
    fn from(value: &PyAgentType) -> AgentType {
        AgentType::from_strings(&value.organization, &value.namespace, &value.agent_type)
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyAgentType {
    #[new]
    pub fn new(agent_org: String, agent_ns: String, agent_class: String) -> Self {
        PyAgentType {
            organization: agent_org,
            namespace: agent_ns,
            agent_type: agent_class,
        }
    }
}

async fn init_tracing_impl(config: TracingConfiguration) -> Result<(), slim_tracing::ConfigError> {
    static TRACING_GUARD: OnceCell<slim_tracing::OtelGuard> = OnceCell::const_new();

    let _ = TRACING_GUARD
        .get_or_init(|| async { config.setup_tracing_subscriber().unwrap() })
        .await;

    Ok(())
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (config))]
pub fn init_tracing(py: Python, config: Py<PyDict>) -> PyResult<Bound<PyAny>> {
    let config: TracingConfiguration = from_pyobject(config.into_bound(py))?;

    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        init_tracing_impl(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}
