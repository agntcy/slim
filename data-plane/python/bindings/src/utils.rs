// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::hash_map::DefaultHasher;
use std::fmt::Display;
use std::hash::Hash;
use std::hash::Hasher;

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

/// name class
#[gen_stub_pyclass]
#[pyclass(name = "Name", eq, str)]
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

impl Display for PyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyName {
    #[new]
    #[pyo3(signature = (component0, component1, component2, id=None))]
    pub fn new(
        component0: String,
        component1: String,
        component2: String,
        id: Option<u64>,
    ) -> Self {
        let name = Name::from_strings([&component0, &component1, &component2]);

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

    pub fn components(&self) -> Vec<u64> {
        self.name.components().to_vec()
    }

    pub fn components_strings(&self) -> Vec<String> {
        self.name.components_strings().to_vec()
    }

    pub fn equal_without_id(&self, name: &PyName) -> bool {
        self.name.components()[0] == name.name.components()[0]
            && self.name.components()[1] == name.name.components()[1]
            && self.name.components()[2] == name.name.components()[2]
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.name.components()[0].hash(&mut hasher);
        self.name.components()[1].hash(&mut hasher);
        self.name.components()[2].hash(&mut hasher);
        self.name.components()[3].hash(&mut hasher);
        hasher.finish()
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
