// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use serde_pyobject::from_pyobject;
use slim_auth::traits::TokenProvider;
use slim_auth::traits::Verifier;

use slim_datapath::messages::encoder::Name;
use slim_session::SessionConfig;
use slim_uniffi::BindingsAdapter;

use crate::pyidentity::IdentityProvider;
use crate::pyidentity::IdentityVerifier;
use crate::pyidentity::PyIdentityProvider;
use crate::pyidentity::PyIdentityVerifier;

use crate::pysession::{PyCompletionHandle, PySessionConfiguration, PySessionContext};
use crate::utils::PyName;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

/// Helper to convert PyName to FFI Name
fn py_name_to_ffi(py_name: &PyName) -> slim_uniffi::Name {
    let internal_name: Name = py_name.into();
    slim_uniffi::Name {
        components: internal_name
            .components_strings()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        id: Some(internal_name.id()),
    }
}

#[gen_stub_pyclass]
#[pyclass(name = "App")]
#[derive(Clone)]
pub struct PyApp {
    internal: Arc<PyAppInternal>,
}

struct PyAppInternal {
    /// The adapter instance (uses AuthProvider/AuthVerifier enums internally)
    /// The adapter manages the service internally
    adapter: Arc<BindingsAdapter>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyApp {
    #[new]
    fn new(
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
        local_service: bool,
    ) -> PyResult<Self> {
        let adapter = pyo3_async_runtimes::tokio::get_runtime()
            .block_on(async move {
                // Convert the PyIdentityProvider into IdentityProvider
                let mut provider: IdentityProvider = provider.into();

                // Initialize the identity provider
                provider
                    .initialize()
                    .await
                    .map_err(|e| format!("Failed to initialize provider: {}", e))?;

                // Convert the PyIdentityVerifier into IdentityVerifier
                let mut verifier: IdentityVerifier = verifier.into();

                // Initialize the identity verifier
                verifier
                    .initialize()
                    .await
                    .map_err(|e| format!("Failed to initialize verifier: {}", e))?;

                // Convert PyName into Name
                let base_name: Name = name.into();

                // IdentityProvider/IdentityVerifier are already AuthProvider/AuthVerifier type aliases
                // Use BindingsAdapter's complete creation logic
                BindingsAdapter::new(base_name, provider, verifier, local_service)
                    .map_err(|e| format!("Failed to create BindingsAdapter: {}", e))
            })
            .map_err(|e: String| PyErr::new::<PyException, _>(e))?;

        let internal = Arc::new(PyAppInternal { adapter });

        Ok(PyApp { internal })
    }

    #[getter]
    pub fn id(&self) -> u64 {
        self.internal.adapter.id()
    }

    #[getter]
    pub fn name(&self) -> PyName {
        // adapter.name() returns slim_uniffi::Name, convert to PyName
        let ffi_name = self.internal.adapter.name();
        // Convert FFI Name back to datapath Name, then to PyName
        let components: [String; 3] = [
            ffi_name.components.first().cloned().unwrap_or_default(),
            ffi_name.components.get(1).cloned().unwrap_or_default(),
            ffi_name.components.get(2).cloned().unwrap_or_default(),
        ];
        let mut datapath_name = Name::from_strings(components);
        if let Some(id) = ffi_name.id {
            datapath_name = datapath_name.with_id(id);
        }
        PyName::from(datapath_name)
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable[tuple[SessionContext, CompletionHandle]]", imports=("collections.abc",)))]
    fn create_session<'a>(
        &'a self,
        py: Python<'a>,
        destination: PyName,
        config: PySessionConfiguration,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Convert to internal types (not FFI types)
            let internal_config = SessionConfig::from(&config);
            let internal_dest: Name = (&destination).into();

            let (session_ctx, completion) = internal_clone
                .adapter
                .create_session_internal(internal_config, internal_dest)
                .await
                .map_err(|e| {
                    PyErr::new::<PyException, _>(format!("Failed to create session: {}", e))
                })?;

            let py_session_ctx = PySessionContext::from(session_ctx);
            let py_completion = PyCompletionHandle::from(completion);

            Ok((py_session_ctx, py_completion))
        })
    }

    #[pyo3(signature = (timeout=None))]
    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable[SessionContext]", imports=("collections.abc",)))]
    fn listen_for_session<'a>(
        &'a self,
        py: Python<'a>,
        timeout: Option<std::time::Duration>,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            internal_clone
                .adapter
                .listen_for_session_internal(timeout)
                .await
                .map_err(|e| {
                    PyErr::new::<PyException, _>(format!("Failed to listen for session: {}", e))
                })
                .map(PySessionContext::from)
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn run_server<'a>(&'a self, py: Python<'a>, config: Py<PyDict>) -> PyResult<Bound<'a, PyAny>> {
        let config: PyGrpcServerConfig = from_pyobject(config.into_bound(py))?;
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Convert to FFI ServerConfig
            let ffi_config = slim_uniffi::ServerConfig {
                endpoint: config.endpoint,
                tls: slim_uniffi::TlsConfig {
                    insecure: config.tls_setting.insecure,
                    insecure_skip_verify: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                    tls_version: None,
                    include_system_ca_certs_pool: None,
                },
            };

            internal_clone
                .adapter
                .run_server_async(ffi_config)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn stop_server<'a>(&'a self, py: Python<'a>, endpoint: String) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            internal_clone
                .adapter
                .stop_server(endpoint)
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable[int]", imports=("collections.abc",)))]
    fn connect<'a>(&'a self, py: Python<'a>, config: Py<PyDict>) -> PyResult<Bound<'a, PyAny>> {
        let config: PyGrpcClientConfig = from_pyobject(config.into_bound(py))?;
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Convert to FFI ClientConfig
            let ffi_config = slim_uniffi::ClientConfig {
                endpoint: config.endpoint,
                tls: slim_uniffi::TlsConfig {
                    insecure: config.tls_setting.insecure,
                    insecure_skip_verify: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                    tls_version: None,
                    include_system_ca_certs_pool: None,
                },
            };

            internal_clone
                .adapter
                .connect_async(ffi_config)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn disconnect<'a>(&'a self, py: Python<'a>, conn: u64) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            internal_clone
                .adapter
                .disconnect_async(conn)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[pyo3(signature = (name, conn=None))]
    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn subscribe<'a>(
        &'a self,
        py: Python<'a>,
        name: PyName,
        conn: Option<u64>,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ffi_name = py_name_to_ffi(&name);

            internal_clone
                .adapter
                .subscribe_async(ffi_name, conn)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[pyo3(signature = (name, conn=None))]
    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn unsubscribe<'a>(
        &'a self,
        py: Python<'a>,
        name: PyName,
        conn: Option<u64>,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ffi_name = py_name_to_ffi(&name);

            internal_clone
                .adapter
                .unsubscribe_async(ffi_name, conn)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn set_route<'a>(
        &'a self,
        py: Python<'a>,
        name: PyName,
        conn: u64,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ffi_name = py_name_to_ffi(&name);

            internal_clone
                .adapter
                .set_route_async(ffi_name, conn)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn remove_route<'a>(
        &'a self,
        py: Python<'a>,
        name: PyName,
        conn: u64,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let ffi_name = py_name_to_ffi(&name);

            internal_clone
                .adapter
                .remove_route_async(ffi_name, conn)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable[CompletionHandle]", imports=("collections.abc",)))]
    fn delete_session<'a>(
        &'a self,
        py: Python<'a>,
        session_context: PySessionContext,
    ) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            session_context
                .delete(&internal_clone.adapter)
                .await
                .map(PyCompletionHandle::from)
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }
}
