// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use serde_pyobject::from_pyobject;
use slim_auth::traits::TokenProvider;
use slim_auth::traits::Verifier;

use slim_datapath::messages::encoder::Name;
use slim_service::bindings::BindingsAdapter;
use slim_service::{ServiceError, ServiceRef};
use slim_session::{SessionConfig, SessionError};

use crate::pyidentity::IdentityProvider;
use crate::pyidentity::IdentityVerifier;
use crate::pyidentity::PyIdentityProvider;
use crate::pyidentity::PyIdentityVerifier;

use crate::pysession::{PySessionConfiguration, PySessionContext};
use crate::utils::PyName;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

#[gen_stub_pyclass]
#[pyclass(name = "App")]
#[derive(Clone)]
pub struct PyApp {
    internal: Arc<PyAppInternal<IdentityProvider, IdentityVerifier>>,
}

struct PyAppInternal<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// The adapter instance
    adapter: BindingsAdapter<P, V>,

    /// Reference to the service
    service: ServiceRef,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyApp {
    #[getter]
    pub fn id(&self) -> u64 {
        self.internal.adapter.id()
    }

    #[getter]
    pub fn name(&self) -> PyName {
        PyName::from(self.internal.adapter.name().clone())
    }
}

impl PyApp {
    async fn new(
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
        local_service: bool,
    ) -> Result<Self, ServiceError> {
        // Convert the PyIdentityProvider into IdentityProvider
        let provider: IdentityProvider = provider.into();

        // Convert the PyIdentityVerifier into IdentityVerifier
        let verifier: IdentityVerifier = verifier.into();

        let base_name: Name = name.into();

        // Use BindingsAdapter's complete creation logic
        let (adapter, service_ref) =
            BindingsAdapter::new(base_name, provider, verifier, local_service).await?;

        // create the service
        let internal = Arc::new(PyAppInternal {
            service: service_ref,
            adapter,
        });

        Ok(PyApp { internal })
    }

    async fn create_session(
        &self,
        destination: Name,
        session_config: SessionConfig,
    ) -> Result<PySessionContext, SessionError> {
        let ctx = self
            .internal
            .adapter
            .create_session(session_config, destination)
            .await?;
        Ok(PySessionContext::from(ctx))
    }

    // Start listening for messages for a specific session id.
    async fn listen_for_session(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<PySessionContext, ServiceError> {
        // Use adapter's listen_for_session method
        let ctx = self.internal.adapter.listen_for_session(timeout).await?;
        Ok(PySessionContext::from(ctx))
    }

    async fn run_server(&self, config: PyGrpcServerConfig) -> Result<(), ServiceError> {
        self.internal
            .service
            .get_service()
            .run_server(&config)
            .await
    }

    async fn stop_server(&self, endpoint: &str) -> Result<(), ServiceError> {
        self.internal.service.get_service().stop_server(endpoint)
    }

    async fn connect(&self, config: PyGrpcClientConfig) -> Result<u64, ServiceError> {
        // Get service and connect
        self.internal.service.get_service().connect(&config).await
    }

    async fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        self.internal.service.get_service().disconnect(conn)
    }

    async fn subscribe(&self, name: PyName, conn: Option<u64>) -> Result<(), ServiceError> {
        self.internal.adapter.subscribe(&name.into(), conn).await
    }

    async fn unsubscribe(&self, name: PyName, conn: Option<u64>) -> Result<(), ServiceError> {
        self.internal.adapter.unsubscribe(&name.into(), conn).await
    }

    async fn set_route(&self, name: PyName, conn: u64) -> Result<(), ServiceError> {
        self.internal.adapter.set_route(&name.into(), conn).await
    }

    async fn remove_route(&self, name: PyName, conn: u64) -> Result<(), ServiceError> {
        self.internal.adapter.remove_route(&name.into(), conn).await
    }

    async fn delete_session(&self, session: PySessionContext) -> Result<(), SessionError> {
        session
            .delete(&self.internal.adapter)
            .await
            .map_err(|e| SessionError::SessionClosed(e.to_string()))
    }
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, destination, config))]
pub fn create_session(
    py: Python,
    svc: PyApp,
    destination: PyName,
    config: PySessionConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.create_session(destination.into(), SessionConfig::from(&config))
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_context))]
pub fn delete_session(
    py: Python,
    svc: PyApp,
    session_context: PySessionContext,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.delete_session(session_context)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc, config,
))]
pub fn run_server(py: Python, svc: PyApp, config: Py<PyDict>) -> PyResult<Bound<PyAny>> {
    let config: PyGrpcServerConfig = from_pyobject(config.into_bound(py))?;

    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.run_server(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc,
    endpoint,
))]
pub fn stop_server(py: Python, svc: PyApp, endpoint: String) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.stop_server(&endpoint)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc,
    config
))]
pub fn connect(py: Python, svc: PyApp, config: Py<PyDict>) -> PyResult<Bound<PyAny>> {
    let config: PyGrpcClientConfig = from_pyobject(config.into_bound(py))?;

    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.connect(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
pub fn disconnect(py: Python, svc: PyApp, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.disconnect(conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, name, conn=None))]
pub fn subscribe(
    py: Python,
    svc: PyApp,
    name: PyName,
    conn: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.subscribe(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, name, conn=None))]
pub fn unsubscribe(
    py: Python,
    svc: PyApp,
    name: PyName,
    conn: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.unsubscribe(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, name, conn))]
pub fn set_route(py: Python, svc: PyApp, name: PyName, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_route(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, name, conn))]
pub fn remove_route(py: Python, svc: PyApp, name: PyName, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove_route(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, timeout=None))]
pub fn listen_for_session(
    py: Python,
    svc: PyApp,
    timeout: Option<std::time::Duration>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.listen_for_session(timeout)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction(name = "create_app")]
#[pyo3(signature = (name, provider, verifier, local_service=false))]
pub fn create_pyapp(
    py: Python,
    name: PyName,
    provider: PyIdentityProvider,
    verifier: PyIdentityVerifier,
    local_service: bool,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        PyApp::new(name, provider, verifier, local_service)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}
