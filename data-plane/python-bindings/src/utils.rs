// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use tokio::sync::OnceCell;

use agp_datapath::messages::encoder::AgentType;

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

impl Into<AgentType> for PyAgentType {
    fn into(self) -> AgentType {
        AgentType::from_strings(&self.organization, &self.namespace, &self.agent_type)
    }
}

impl Into<AgentType> for &PyAgentType {
    fn into(self) -> AgentType {
        AgentType::from_strings(&self.organization, &self.namespace, &self.agent_type)
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

async fn init_tracing_impl(log_level: String, enable_opentelemetry: bool) {
    static TRACING_GUARD: OnceCell<agp_tracing::OtelGuard> = OnceCell::const_new();

    let _ = TRACING_GUARD
        .get_or_init(|| async {
            let mut config = agp_tracing::TracingConfiguration::default().with_log_level(log_level);

            if enable_opentelemetry {
                config = config.clone().enable_opentelemetry();
            }

            let otel_guard = config.setup_tracing_subscriber();

            otel_guard
        })
        .await;
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (log_level="info".to_string(), enable_opentelemetry=false,))]
pub fn init_tracing(py: Python, log_level: String, enable_opentelemetry: bool) {
    let _ = pyo3_async_runtimes::tokio::future_into_py(py, async move {
        Ok(init_tracing_impl(log_level, enable_opentelemetry).await)
    });
}
