// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pymethods;

use agp_datapath::messages::encoder::AgentType;

/// agent class
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PyAgentType {
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
