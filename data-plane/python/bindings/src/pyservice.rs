// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

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
use slim_service::session;
use slim_service::session::Notification;
use slim_service::session::SessionError;
use slim_service::{Service, ServiceError};
use tokio::sync::RwLock;

use crate::pyidentity::IdentityProvider;
use crate::pyidentity::IdentityVerifier;
use crate::pyidentity::PyIdentityProvider;
use crate::pyidentity::PyIdentityVerifier;
use crate::pymessage::PyMessageContext;
use crate::pysession::{PySessionConfiguration, PySessionContext};
use crate::utils::PyName;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

// Global static service instance
static GLOBAL_SERVICE: OnceLock<Service> = OnceLock::new();

#[allow(clippy::large_enum_variant)]
enum ServiceRef {
    Global(&'static Service),
    Local(Service),
}

impl ServiceRef {
    fn get_service(&self) -> &Service {
        match self {
            ServiceRef::Global(s) => s,
            ServiceRef::Local(s) => s,
        }
    }
}

// Helper function to get or initialize the global service
fn get_or_init_global_service() -> &'static Service {
    GLOBAL_SERVICE.get_or_init(|| {
        let svc_id = slim_config::component::id::ID::new_with_str("service/0").unwrap();
        Service::new(svc_id)
    })
}

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
    service: ServiceRef,
    name: Name,
    rx: RwLock<
        tokio::sync::mpsc::Receiver<
            Result<Notification<IdentityProvider, IdentityVerifier>, SessionError>,
        >,
    >,
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
        local_service: bool,
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

        // Determine whether to use global or local service
        let service_ref = if local_service {
            // create local service
            let svc = Service::new(svc_id);
            ServiceRef::Local(svc)
        } else {
            // Use global service, initialize if needed
            let global_svc = get_or_init_global_service();
            ServiceRef::Global(global_svc)
        };

        // Get the service reference for creating the app
        let svc = service_ref.get_service();

        // Get the rx channel
        let (app, rx) = svc.create_app(&name, provider, verifier).await?;

        // create the service
        let sdk = Arc::new(PyServiceInternal {
            service: service_ref,
            app,
            name,
            rx: RwLock::new(rx),
        });

        Ok(PyService { sdk })
    }

    async fn create_session(
        &self,
        session_config: session::SessionConfig,
    ) -> Result<PySessionContext, SessionError> {
        let ctx = self.sdk.app.create_session(session_config, None).await?;
        Ok(PySessionContext::from(ctx))
    }

    // Start listening for messages for a specific session id.
    async fn listen_for_session(&self) -> Result<PySessionContext, ServiceError> {
        // Wait for new sessions
        let mut rx = self.sdk.rx.write().await;

        tokio::select! {
            notification = rx.recv() => {
                if notification.is_none() {
                    return Err(ServiceError::ReceiveError("application channel closed".to_string()));
                }

                let notification = notification.unwrap();
                match notification {
                    Ok(Notification::NewSession(ctx)) => {
                        Ok(PySessionContext::from(ctx))
                    }
                    Ok(Notification::NewMessage(m)) => {
                        Err(ServiceError::ReceiveError(format!("receive unexpected message from app channel: {:?}", m)))
                    }
                    Err(e) => {
                        Err(ServiceError::ReceiveError(format!("failed to receive notification: {}", e)))
                    }
                }

            }
        }
    }

    async fn get_message(
        &self,
        session_context: PySessionContext,
    ) -> Result<(PyMessageContext, Vec<u8>), ServiceError> {
        // Acquire rx lock and wait for new messages on the session
        let mut rx = session_context.internal.rx.write().await;

        tokio::select! {
            msg = rx.recv() => {
                if msg.is_none() {
                    return Err(ServiceError::ReceiveError("application channel closed".to_string()));
                }

                let msg = msg.unwrap().map_err(|e| ServiceError::ReceiveError(format!("failed to decode message: {}", e)))?;
                Ok(PyMessageContext::from_proto_message(msg)?)
            }
        }
    }

    async fn delete_session(&self, session: PySessionContext) -> Result<(), SessionError> {
        // Get an Arc to the session
        let session = session
            .internal
            .session
            .upgrade()
            .ok_or(SessionError::SessionClosed(
                "session already closed".to_string(),
            ))?;

        self.sdk.app.delete_session(&session).await
    }

    async fn run_server(&self, config: PyGrpcServerConfig) -> Result<(), ServiceError> {
        self.sdk.service.get_service().run_server(&config)
    }

    async fn stop_server(&self, endpoint: &str) -> Result<(), ServiceError> {
        self.sdk.service.get_service().stop_server(endpoint)
    }

    async fn connect(&self, config: PyGrpcClientConfig) -> Result<u64, ServiceError> {
        // Get service and connect
        self.sdk.service.get_service().connect(&config).await
    }

    async fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        self.sdk.service.get_service().disconnect(conn)
    }

    async fn subscribe(&self, name: PyName, conn: Option<u64>) -> Result<(), ServiceError> {
        self.sdk.app.subscribe(&name.into(), conn).await
    }

    async fn unsubscribe(&self, name: PyName, conn: Option<u64>) -> Result<(), ServiceError> {
        self.sdk.app.unsubscribe(&name.into(), conn).await
    }

    async fn set_route(&self, name: PyName, conn: u64) -> Result<(), ServiceError> {
        self.sdk.app.set_route(&name.into(), conn).await
    }

    async fn remove_route(&self, name: PyName, conn: u64) -> Result<(), ServiceError> {
        self.sdk.app.remove_route(&name.into(), conn).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish(
        &self,
        session_ctx: PySessionContext,
        fanout: u32,
        blob: Vec<u8>,
        message_ctx: Option<PyMessageContext>,
        name: Option<PyName>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        let session = session_ctx
            .internal
            .session
            .upgrade()
            .ok_or(ServiceError::SessionError("session closed".to_string()))?;

        let (name, conn_out) = match &name {
            Some(name) => (name, None),
            None => match &message_ctx {
                Some(ctx) => (&ctx.source_name, Some(ctx.input_connection)),
                None => match session.session_config().destination_name() {
                    Some(n) => (&PyName::from(n), None),
                    None => {
                        return Err(ServiceError::SessionError(
                            "either name or message_ctx must be provided for publish".to_string(),
                        ));
                    }
                },
            },
        };

        let name = Name::from(name);

        // set flags
        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        session
            .publish_with_flags(&name, flags, blob, payload_type, metadata)
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    async fn invite(
        &self,
        session_context: PySessionContext,
        name: PyName,
    ) -> Result<(), ServiceError> {
        session_context
            .internal
            .session
            .upgrade()
            .ok_or(ServiceError::SessionError("session closed".to_string()))?
            .invite_participant(&name.into())
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    async fn remove(
        &self,
        session_context: PySessionContext,
        name: PyName,
    ) -> Result<(), ServiceError> {
        session_context
            .internal
            .session
            .upgrade()
            .ok_or(ServiceError::SessionError("session closed".to_string()))?
            .remove_participant(&name.into())
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    fn set_default_session_config(
        &self,
        config: session::SessionConfig,
    ) -> Result<(), SessionError> {
        self.sdk.app.set_default_session_config(&config)
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
#[pyo3(signature = (svc, session_context))]
pub fn delete_session(
    py: Python,
    svc: PyService,
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
#[pyo3(signature = (svc, config))]
pub fn set_default_session_config(
    _py: Python,
    svc: PyService,
    config: PySessionConfiguration,
) -> PyResult<()> {
    svc.set_default_session_config(config.into())
        .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
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
#[pyo3(signature = (svc, name, conn=None))]
pub fn subscribe(
    py: Python,
    svc: PyService,
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
    svc: PyService,
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
pub fn set_route(py: Python, svc: PyService, name: PyName, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_route(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, name, conn))]
pub fn remove_route(py: Python, svc: PyService, name: PyName, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove_route(name, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[allow(clippy::too_many_arguments)]
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_context, fanout, blob, message_ctx=None, name=None, payload_type=None, metadata=None))]
pub fn publish(
    py: Python,
    svc: PyService,
    session_context: PySessionContext,
    fanout: u32,
    blob: Vec<u8>,
    message_ctx: Option<PyMessageContext>,
    name: Option<PyName>,
    payload_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.publish(
            session_context,
            fanout,
            blob,
            message_ctx,
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
#[pyo3(signature = (svc, session_context, name))]
pub fn invite(
    py: Python,
    svc: PyService,
    session_context: PySessionContext,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.invite(session_context, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_context, name))]
pub fn remove(
    py: Python,
    svc: PyService,
    session_context: PySessionContext,
    name: PyName,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove(session_context, name)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc))]
pub fn listen_for_session(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.listen_for_session()
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_context))]
pub fn get_message(
    py: Python,
    svc: PyService,
    session_context: PySessionContext,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py_with_locals(
        py,
        pyo3_async_runtimes::tokio::get_current_locals(py)?,
        async move {
            svc.get_message(session_context)
                .await
                .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
        },
    )
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (name, provider, verifier, local_service=false))]
pub fn create_pyservice(
    py: Python,
    name: PyName,
    provider: PyIdentityProvider,
    verifier: PyIdentityVerifier,
    local_service: bool,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        PyService::create_pyservice(name, provider, verifier, local_service)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}
