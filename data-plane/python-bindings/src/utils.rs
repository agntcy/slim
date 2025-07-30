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

use slim_datapath::messages::encoder::Name;

/// agent class
#[gen_stub_pyclass]
#[pyclass(eq)]
#[derive(Clone, PartialEq)]
pub struct PyName {
    name: Name,
}

impl From<PyName> for Name {
    fn from(value: PyName) -> Name {
        value.name
    }
}

impl From<&PyName> for Name {
    fn from(value: &PyName) -> Name {
        value.name.clone()
    }
}

impl From<Name> for PyName {
    fn from(name: Name) -> Self {
        PyName { name }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyName {
    #[new]
    #[pyo3(signature = (agent_org, agent_ns, agent_class, id=None))]
    pub fn new(agent_org: String, agent_ns: String, agent_class: String, id: Option<u64>) -> Self {
        let name = Name::from_strings([&agent_org, &agent_ns, &agent_class]);

        PyName {
            name: match id {
                Some(id) => name.with_id(id),
                None => name,
            },
        }
    }

    #[getter]
    pub fn id(&self) -> u64 {
        self.name.id()
    }

    #[setter]
    pub fn set_id(&mut self, id: u64) {
        self.name.set_id(id);
    }

    #[getter]
    pub fn organization(&self) -> String {
        self.name.components_strings().unwrap()[0].to_string()
    }

    #[getter]
    pub fn namespace(&self) -> String {
        self.name.components_strings().unwrap()[1].to_string()
    }

    #[getter]
    pub fn agent_type(&self) -> String {
        self.name.components_strings().unwrap()[2].to_string()
    }

    fn __repr__(&self) -> String {
        self.name.to_string()
    }

    fn __str__(&self) -> String {
        self.name.to_string()
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
