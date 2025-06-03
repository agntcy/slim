use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use rand::Rng;
use serde_pyobject::from_pyobject;
use slim_datapath::messages::encoder::{Agent, AgentType};
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_service::session;
use slim_service::{Service, ServiceError};
use tokio::sync::RwLock;

use crate::pysession::PySessionType;
use crate::pysession::{PySessionConfiguration, PySessionInfo};
use crate::utils::PyAgentType;
use slim_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use slim_config::grpc::server::ServerConfig as PyGrpcServerConfig;

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub struct PyService {
    sdk: Arc<PyServiceInternal>,
}

struct PyServiceInternal {
    service: Service,
    agent: Agent,
    rx: RwLock<session::AppChannelReceiver>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyService {
    #[getter]
    pub fn id(&self) -> u64 {
        self.sdk.agent.agent_id()
    }
}

impl PyService {
    async fn create_pyservice(
        organization: String,
        namespace: String,
        agent_type: String,
        id: Option<u64>,
    ) -> Result<Self, ServiceError> {
        let id = match id {
            Some(v) => v,
            None => {
                let mut rng = rand::rng();
                rng.random()
            }
        };

        // create local agent
        let agent = Agent::from_strings(&organization, &namespace, &agent_type, id);

        // create service ID
        let svc_id = slim_config::component::id::ID::new_with_str("service/0").unwrap();

        // create local service
        let svc = Service::new(svc_id);

        // Get the rx channel
        let rx = svc.create_agent(&agent).await?;

        // create the service
        let sdk = Arc::new(PyServiceInternal {
            service: svc,
            agent,
            rx: RwLock::new(rx),
        });

        Ok(PyService { sdk })
    }

    async fn create_session(
        &self,
        session_config: session::SessionConfig,
    ) -> Result<PySessionInfo, ServiceError> {
        Ok(PySessionInfo::from(
            self.sdk
                .service
                .create_session(&self.sdk.agent, session_config)
                .await?,
        ))
    }

    async fn delete_session(&self, session_id: session::Id) -> Result<(), ServiceError> {
        self.sdk
            .service
            .delete_session(&self.sdk.agent, session_id)
            .await
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

    async fn subscribe(
        &self,
        conn: u64,
        name: PyAgentType,
        id: Option<u64>,
    ) -> Result<(), ServiceError> {
        let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);

        self.sdk
            .service
            .subscribe(&self.sdk.agent, &class, id, Some(conn))
            .await
    }

    async fn unsubscribe(
        &self,
        conn: u64,
        name: PyAgentType,
        id: Option<u64>,
    ) -> Result<(), ServiceError> {
        let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
        self.sdk
            .service
            .unsubscribe(&self.sdk.agent, &class, id, Some(conn))
            .await
    }

    async fn set_route(
        &self,
        conn: u64,
        name: PyAgentType,
        id: Option<u64>,
    ) -> Result<(), ServiceError> {
        let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
        self.sdk
            .service
            .set_route(&self.sdk.agent, &class, id, conn)
            .await
    }

    async fn remove_route(
        &self,
        conn: u64,
        name: PyAgentType,
        id: Option<u64>,
    ) -> Result<(), ServiceError> {
        let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
        self.sdk
            .service
            .remove_route(&self.sdk.agent, &class, id, conn)
            .await
    }

    async fn publish(
        &self,
        session_info: session::Info,
        fanout: u32,
        blob: Vec<u8>,
        name: Option<PyAgentType>,
        id: Option<u64>,
    ) -> Result<(), ServiceError> {
        let (agent_type, agent_id, conn_out) = match name {
            Some(name) => (name.into(), id, None),
            None => {
                // use the session_info to set a name
                match &session_info.message_source {
                    Some(agent) => (
                        agent.agent_type().clone(),
                        Some(agent.agent_id()),
                        session_info.input_connection,
                    ),
                    None => {
                        return Err(ServiceError::ConfigError("no agent specified".to_string()));
                    }
                }
            }
        };

        // set flags
        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        self.sdk
            .service
            .publish_with_flags(
                &self.sdk.agent,
                session_info,
                &agent_type,
                agent_id,
                flags,
                blob,
            )
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

                // extract agent and payload
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
    ) -> Result<(), ServiceError> {
        self.sdk
            .service
            .set_session_config(&self.sdk.agent, &config, Some(session_id))
            .await
    }

    async fn get_session_config(
        &self,
        session_id: u32,
    ) -> Result<PySessionConfiguration, ServiceError> {
        self.sdk
            .service
            .get_session_config(&self.sdk.agent, session_id)
            .await
            .map(|val| val.into())
    }

    async fn set_default_session_config(
        &self,
        config: session::SessionConfig,
    ) -> Result<(), ServiceError> {
        self.sdk
            .service
            .set_session_config(&self.sdk.agent, &config, None)
            .await
    }

    async fn get_default_session_config(
        &self,
        session_type: session::SessionType,
    ) -> Result<PySessionConfiguration, ServiceError> {
        self.sdk
            .service
            .get_default_session_config(&self.sdk.agent, session_type)
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
#[pyo3(signature = (svc, conn, name, id=None))]
pub fn subscribe(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.subscribe(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
pub fn unsubscribe(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.unsubscribe(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
pub fn set_route(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.set_route(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
pub fn remove_route(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.remove_route(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_info, fanout, blob, name=None, id=None))]
pub fn publish(
    py: Python,
    svc: PyService,
    session_info: PySessionInfo,
    fanout: u32,
    blob: Vec<u8>,
    name: Option<PyAgentType>,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.publish(session_info.session_info, fanout, blob, name, id)
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
#[pyo3(signature = (organization, namespace, agent_type, id=None))]
pub fn create_pyservice(
    py: Python,
    organization: String,
    namespace: String,
    agent_type: String,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        PyService::create_pyservice(organization, namespace, agent_type, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(e.to_string()))
    })
}
