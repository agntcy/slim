// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use serde_pyobject::from_pyobject;
use slim_bindings::{
    BindingsAdapter, ClientJwtAuth, IdentityProviderConfig, IdentityVerifierConfig, JwtAuth,
    JwtKeyConfig, JwtKeyType, Service as BindingsService, SlimError, StaticJwtAuth,
    get_or_init_global_service,
};
use slim_datapath::messages::encoder::Name;
use slim_session::SessionConfig;

use crate::pyidentity::PyIdentityProvider;
use crate::pyidentity::PyIdentityVerifier;

use crate::pysession::{PyCompletionHandle, PySessionConfiguration, PySessionContext};
use crate::utils::PyName;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

#[gen_stub_pyclass]
#[pyclass(name = "App")]
#[derive(Clone)]
pub struct PyApp {
    internal: Arc<PyAppInternal>,
}

struct PyAppInternal {
    /// The adapter instance (uses AuthProvider/AuthVerifier enums internally)
    adapter: Arc<BindingsAdapter>,
    /// The service instance for service-level operations (run_server, connect, etc.)
    service: Arc<BindingsService>,
}

/// Helper function to convert PyName to Arc<FfiName>
fn py_name_to_ffi(py_name: &PyName) -> Arc<slim_bindings::Name> {
    let ffi_name: slim_bindings::Name = py_name.into();
    Arc::new(ffi_name)
}

#[gen_stub_pymethods]
#[pymethods]
impl PyApp {
    #[new]
    #[pyo3(signature = (name, provider, verifier, local_service=false))]
    fn new(
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
        local_service: bool,
    ) -> PyResult<Self> {
        async fn create_adapter(
            name: PyName,
            provider: PyIdentityProvider,
            verifier: PyIdentityVerifier,
            local_service: bool,
        ) -> Result<(Arc<BindingsAdapter>, Arc<BindingsService>), SlimError> {
            // Convert PyIdentityProvider to IdentityProviderConfig
            let provider_config: IdentityProviderConfig = match provider {
                PyIdentityProvider::StaticJwt { path } => IdentityProviderConfig::StaticJwt {
                    config: StaticJwtAuth {
                        token_file: path,
                        duration: std::time::Duration::from_secs(3600),
                    },
                },
                PyIdentityProvider::Jwt {
                    private_key,
                    duration,
                    issuer,
                    audience,
                    subject,
                } => {
                    // Convert PyKey to slim_auth::jwt::Key first, then to JwtKeyConfig
                    let auth_key: slim_auth::jwt::Key = private_key.into();
                    let key_config = JwtKeyConfig {
                        algorithm: auth_key.algorithm.into(),
                        format: auth_key.format.into(),
                        key: auth_key.key.into(),
                    };
                    IdentityProviderConfig::Jwt {
                        config: ClientJwtAuth {
                            key: JwtKeyType::Encoding { key: key_config },
                            audience,
                            issuer,
                            subject,
                            duration,
                        },
                    }
                }
                PyIdentityProvider::SharedSecret {
                    identity,
                    shared_secret,
                } => IdentityProviderConfig::SharedSecret {
                    id: identity,
                    data: shared_secret,
                },
                #[cfg(not(target_family = "windows"))]
                PyIdentityProvider::Spire {
                    socket_path,
                    target_spiffe_id,
                    jwt_audiences,
                } => {
                    use slim_bindings::SpireConfig;
                    IdentityProviderConfig::Spire {
                        config: SpireConfig {
                            socket_path,
                            target_spiffe_id,
                            jwt_audiences: jwt_audiences
                                .unwrap_or_else(|| vec!["slim".to_string()]),
                            trust_domains: vec![],
                        },
                    }
                }
                #[cfg(target_family = "windows")]
                PyIdentityProvider::Spire { .. } => {
                    return Err(SlimError::Auth(
                        slim_auth::AuthError::SpireUnsupportedOnWindows,
                    ));
                }
            };

            // Convert PyIdentityVerifier to IdentityVerifierConfig
            let verifier_config: IdentityVerifierConfig = match verifier {
                PyIdentityVerifier::Jwt {
                    public_key,
                    autoresolve,
                    issuer,
                    audience,
                    subject,
                    ..
                } => {
                    let key_type = if autoresolve {
                        JwtKeyType::Autoresolve
                    } else if let Some(key) = public_key {
                        // Convert PyKey to slim_auth::jwt::Key first, then to JwtKeyConfig
                        let auth_key: slim_auth::jwt::Key = key.into();
                        let key_config = JwtKeyConfig {
                            algorithm: auth_key.algorithm.into(),
                            format: auth_key.format.into(),
                            key: auth_key.key.into(),
                        };
                        JwtKeyType::Decoding { key: key_config }
                    } else {
                        JwtKeyType::Autoresolve
                    };

                    IdentityVerifierConfig::Jwt {
                        config: JwtAuth {
                            key: key_type,
                            audience,
                            issuer,
                            subject,
                            duration: std::time::Duration::from_secs(3600),
                        },
                    }
                }
                PyIdentityVerifier::SharedSecret {
                    identity,
                    shared_secret,
                } => IdentityVerifierConfig::SharedSecret {
                    id: identity,
                    data: shared_secret,
                },
                #[cfg(not(target_family = "windows"))]
                PyIdentityVerifier::Spire {
                    socket_path,
                    target_spiffe_id,
                    jwt_audiences,
                } => {
                    use slim_bindings::SpireConfig;
                    IdentityVerifierConfig::Spire {
                        config: SpireConfig {
                            socket_path,
                            target_spiffe_id,
                            jwt_audiences: jwt_audiences
                                .unwrap_or_else(|| vec!["slim".to_string()]),
                            trust_domains: vec![],
                        },
                    }
                }
                #[cfg(target_family = "windows")]
                PyIdentityVerifier::Spire { .. } => {
                    return Err(SlimError::Auth(
                        slim_auth::AuthError::SpireUnsupportedOnWindows,
                    ));
                }
            };

            // Convert PyName to slim_datapath::messages::Name (SlimName)
            let slim_name: slim_datapath::messages::Name = name.into();

            // Create service based on local_service parameter
            let service_instance = if local_service {
                // Create a local service instance
                Arc::new(BindingsService::new("localservice".to_string()))
            } else {
                // Use global service
                get_or_init_global_service()
            };

            // Use BindingsAdapter's async constructor with optional service
            let adapter =
                BindingsAdapter::new_async_with_service(
                    slim_name,
                    provider_config,
                    verifier_config,
                    Some(service_instance.inner())
                ).await?;

            Ok((Arc::new(adapter), service_instance))
        }

        let (adapter, service) = pyo3_async_runtimes::tokio::get_runtime()
            .block_on(create_adapter(name, provider, verifier, local_service))
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))?;

        let internal = Arc::new(PyAppInternal { adapter, service });

        Ok(PyApp { internal })
    }

    #[getter]
    pub fn id(&self) -> u64 {
        self.internal.adapter.id()
    }

    #[getter]
    pub fn name(&self) -> PyName {
        // adapter.name() returns slim_bindings::Name, convert to PyName
        self.internal.adapter.name().as_ref().into()
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
            let ffi_config = slim_bindings::ServerConfig {
                endpoint: config.endpoint,
                tls: slim_bindings::TlsServerConfig {
                    insecure: config.tls_setting.insecure,
                    source: slim_bindings::TlsSource::None,
                    client_ca: slim_bindings::CaSource::None,
                    include_system_ca_certs_pool: true,
                    tls_version: "tls1.3".to_string(),
                    reload_client_ca_file: false,
                },
                http2_only: true,
                max_frame_size: None,
                max_concurrent_streams: None,
                max_header_list_size: None,
                read_buffer_size: None,
                write_buffer_size: None,
                keepalive: slim_bindings::KeepaliveServerParameters::default(),
                auth: slim_bindings::ServerAuthenticationConfig::None,
                metadata: None,
            };

            internal_clone
                .service
                .run_server(ffi_config)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn stop_server<'a>(&'a self, py: Python<'a>, endpoint: String) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            internal_clone
                .service
                .stop_server(endpoint)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable[int]", imports=("collections.abc",)))]
    fn connect<'a>(&'a self, py: Python<'a>, config: Py<PyDict>) -> PyResult<Bound<'a, PyAny>> {
        let config: PyGrpcClientConfig = from_pyobject(config.into_bound(py))?;
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Convert to FFI ClientConfig
            let ffi_config = slim_bindings::ClientConfig {
                endpoint: config.endpoint,
                origin: None,
                server_name: None,
                compression: None,
                rate_limit: None,
                tls: slim_bindings::TlsClientConfig {
                    insecure: config.tls_setting.insecure,
                    insecure_skip_verify: false,
                    source: slim_bindings::TlsSource::None,
                    ca_source: slim_bindings::CaSource::None,
                    include_system_ca_certs_pool: true,
                    tls_version: "tls1.3".to_string(),
                },
                keepalive: None,
                proxy: slim_bindings::ProxyConfig::default(),
                connect_timeout: std::time::Duration::from_secs(10),
                request_timeout: std::time::Duration::from_secs(30),
                buffer_size: None,
                headers: std::collections::HashMap::new(),
                auth: slim_bindings::ClientAuthenticationConfig::None,
                backoff: slim_bindings::BackoffConfig::Exponential {
                    config: slim_bindings::ExponentialBackoff::default(),
                },
                metadata: None,
            };

            internal_clone
                .service
                .connect(ffi_config)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        })
    }

    #[gen_stub(override_return_type(type_repr="collections.abc.Awaitable", imports=("collections.abc",)))]
    fn disconnect<'a>(&'a self, py: Python<'a>, conn: u64) -> PyResult<Bound<'a, PyAny>> {
        let internal_clone = self.internal.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            internal_clone
                .service
                .disconnect(conn)
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
