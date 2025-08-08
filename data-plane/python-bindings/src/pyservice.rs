// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
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
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_service::app::App;
use slim_service::errors::SessionError;
use slim_service::session;
use slim_service::{Service, ServiceError};
use tokio::sync::RwLock;

use crate::pyidentity::IdentityProvider;
use crate::pyidentity::IdentityVerifier;
use crate::pyidentity::PyIdentityProvider;
use crate::pyidentity::PyIdentityVerifier;
use crate::pysession::PySessionType;
use crate::pysession::{PySessionConfiguration, PySessionInfo};
use crate::utils::PyName;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub struct PyService {
    sdk: Arc<PyServiceInternal<IdentityProvider, IdentityVerifier>>,
}

struct PyServiceInternal<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    app: App<P, V>,
    service: Service,
    name: Name,
    rx: RwLock<session::AppChannelReceiver>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyService {
    #[getter]
    pub fn id(&self) -> u64 {
        self.sdk.name.id()
    }

    #[getter]
    pub fn name(&self) -> PyName {
        PyName::from(self.sdk.name.clone())
    }
}

impl PyService {
    async fn create_pyservice(
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
    ) -> Result<Self, ServiceError> {
        // Convert the PyIdentityProvider into IdentityProvider
        let provider: IdentityProvider = provider.into();

        // Convert the PyIdentityVerifier into IdentityVerifier
        let verifier: IdentityVerifier = verifier.into();

        let _identity_token = provider.get_token().map_err(|e| {
            ServiceError::ConfigError(format!("Failed to get token from provider: {}", e))
        })?;

        // TODO(msardara): we can likely get more information from the token here, like a global instance ID
        let name: Name = name.into();
        let name = name.with_id(rand::random::<u64>());

        // create service ID
        let svc_id = slim_config::component::id::ID::new_with_str("service/0").unwrap();

        // create local service
        let svc = Service::new(svc_id);

        // Get the rx channel
        let (app, rx) = svc.create_app(&name, provider, verifier).await?;

        // create the service
        let sdk = Arc::new(PyServiceInternal {
            service: svc,
            app,
            name,
            rx: RwLock::new(rx),
        });

        Ok(PyService { sdk })
    }

    async fn create_session(
        &self,
        session_config: session::SessionConfig,
    ) -> Result<PySessionInfo, SessionError> {
        Ok(PySessionInfo::from(
            self.sdk.app.create_session(session_config, None).await?,
        ))
    }

    async fn delete_session(&self, session_id: session::Id) -> Result<(), SessionError> {
        self.sdk.app.delete_session(session_id).await
    }

    async fn run_server(&self, config: PyGrpcServerConfig) -> Result<(), ServiceError> {
        self.sdk.service.run_server(&config)
    }

    async fn stop_server(&self, endpoint: &str) -> Result<(), ServiceError> {
        self.sdk.service.stop_server(endpoint)
    }

    async fn connect(&self, config: PyGrpcClientConfig) -> Result<u64, ServiceError> {
        // Get service and connect
        self.sdk.service.connect(&config).await
    }

    async fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        self.sdk.service.disconnect(conn)
    }

    async fn subscribe(&self, conn: u64, name: PyName) -> Result<(), ServiceError> {
        self.sdk.app.subscribe(&name.into(), Some(conn)).await
    }

    async fn unsubscribe(&self, conn: u64, name: PyName) -> Result<(), ServiceError> {
        self.sdk.app.unsubscribe(&name.into(), Some(conn)).await
    }

    async fn set_route(&self, conn: u64, name: PyName) -> Result<(), ServiceError> {
        self.sdk.app.set_route(&name.into(), conn).await
    }

    async fn remove_route(&self, conn: u64, name: PyName) -> Result<(), ServiceError> {
        self.sdk.app.remove_route(&name.into(), conn).await
    }

    async fn publish(
        &self,
        session_info: session::Info,
        fanout: u32,
        blob: Vec<u8>,
        name: Option<PyName>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        let (name, conn_out) = match name {
            Some(name) => (name.into(), None),
            None => {
                // use the session_info to set a name
                match &session_info.message_source {
                    Some(name_in_session) => {
                        (name_in_session.clone(), session_info.input_connection)
                    }
                    None => {
                        return Err(ServiceError::ConfigError(
                            "no destination name specified".to_string(),
                        ));
                    }
                }
            }
        };

        // set flags
        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        self.sdk
            .app
            .publish_with_flags(session_info, &name, flags, blob, payload_type, metadata)
            .await
    }

    async fn invite(&self, session_info: session::Info, name: PyName) -> Result<(), ServiceError> {
        self.sdk
            .app
            .invite_participant(&name.into(), session_info)
            .await
    }

    async fn remove(&self, session_info: session::Info, name: PyName) -> Result<(), ServiceError> {
        self.sdk
            .app
            .remove_participant(&name.into(), session_info)
            .await
    }

    async fn receive(&self) -> Result<(PySessionInfo, Vec<u8>), ServiceError> {
        let mut rx = self.sdk.rx.write().await;

        // tokio select
        tokio::select! {
            msg = rx.recv() => {
                if msg.is_none() {
                    return Err(ServiceError::ReceiveError("no message received".to_string()));
                }

                let msg = msg.unwrap().map_err(|e| ServiceError::ReceiveError(e.to_string()))?;

                // extract payload
                let content = match msg.message.message_type {
                    Some(ref msg_type) => match msg_type {
                        slim_datapath::api::ProtoPublishType(publish) => &publish.get_payload().blob,
                        _ => Err(ServiceError::ReceiveError(
                            "receive publish message type".to_string(),
                        ))?,
                    },
                    _ => Err(ServiceError::ReceiveError(
                        "no message received".to_string(),
                    ))?,
                };

                Ok((PySessionInfo::from(msg.info), content.to_vec()))
            }
        }
    }

    async fn set_session_config(
        &self,
        session_id: u32,
        config: session::SessionConfig,
    ) -> Result<(), SessionError> {
        self.sdk
            .app
            .set_session_config(&config, Some(session_id))
            .await
    }

    async fn get_session_config(
        &self,
        session_id: u32,
    ) -> Result<PySessionConfiguration, SessionError> {
        self.sdk
            .app
            .get_session_config(session_id)
            .await
            .map(|val| val.into())
    }

    async fn set_default_session_config(
        &self,
        config: session::SessionConfig,
    ) -> Result<(), SessionError> {
        self.sdk.app.set_session_config(&config, None).await
    }

    async fn get_default_session_config(
        &self,
        session_type: session::SessionType,
    ) -> Result<PySessionConfiguration, SessionError> {
        self.sdk
            .app
            .get_default_session_config(session_type)
            .await
            .map(|val| val.into())
    }
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config))]
pub fn create_session(
    py: Python,
    svc: PyService,
    config: PySessionConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.create_session(config.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_id))]
pub fn delete_session(py: Python, svc: PyService, session_id: u32) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.delete_session(session_id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_id, config))]
pub fn set_session_config(
    py: Python,
    svc: PyService,
    session_id: u32,
    config: PySessionConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_session_config(session_id, config.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_id))]
pub fn get_session_config(py: Python, svc: PyService, session_id: u32) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.get_session_config(session_id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config))]
pub fn set_default_session_config(
    py: Python,
    svc: PyService,
    config: PySessionConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_default_session_config(config.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_type))]
pub fn get_default_session_config(
    py: Python,
    svc: PyService,
    session_type: PySessionType,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.get_default_session_config(session_type.into())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc, config,
))]
pub fn run_server(py: Python, svc: PyService, config: Py<PyDict>) -> PyResult<Bound<PyAny>> {
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
pub fn stop_server(py: Python, svc: PyService, endpoint: String) -> PyResult<Bound<PyAny>> {
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
pub fn connect(py: Python, svc: PyService, config: Py<PyDict>) -> PyResult<Bound<PyAny>> {
    let config: PyGrpcClientConfig = from_pyobject(config.into_bound(py))?;

    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.connect(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
pub fn disconnect(py: Python, svc: PyService, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.disconnect(conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name))]
pub fn subscribe(py: Python, svc: PyService, conn: u64, name: PyName) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.subscribe(conn, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name))]
pub fn unsubscribe(py: Python, svc: PyService, conn: u64, name: PyName) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.unsubscribe(conn, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name))]
pub fn set_route(py: Python, svc: PyService, conn: u64, name: PyName) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_route(conn, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name))]
pub fn remove_route(py: Python, svc: PyService, conn: u64, name: PyName) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove_route(conn, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[allow(clippy::too_many_arguments)]
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_info, fanout, blob, name=None, payload_type=None, metadata=None))]
pub fn publish(
    py: Python,
    svc: PyService,
    session_info: PySessionInfo,
    fanout: u32,
    blob: Vec<u8>,
    name: Option<PyName>,
    payload_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.publish(
            session_info.session_info,
            fanout,
            blob,
            name,
            payload_type,
            metadata,
        )
        .await
        .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_info, name))]
pub fn invite(
    py: Python,
    svc: PyService,
    session_info: PySessionInfo,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.invite(session_info.session_info, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_info, name))]
pub fn remove(
    py: Python,
    svc: PyService,
    session_info: PySessionInfo,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove(session_info.session_info, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc))]
pub fn receive(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py_with_locals(
        py,
        pyo3_async_runtimes::tokio::get_current_locals(py)?,
        async move {
            svc.receive()
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        },
    )
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (name, provider, verifier))]
pub fn create_pyservice(
    py: Python,
    name: PyName,
    provider: PyIdentityProvider,
    verifier: PyIdentityVerifier,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        PyService::create_pyservice(name, provider, verifier)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}
