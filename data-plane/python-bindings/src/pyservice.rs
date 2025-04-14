use std::sync::Arc;

use agp_datapath::messages::encoder::{Agent, AgentType};
use agp_datapath::messages::utils::AgpHeaderFlags;
use agp_service::session;
use agp_service::{Service, ServiceError};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use rand::Rng;
use serde_pyobject::from_pyobject;
use tokio::sync::RwLock;

use crate::pysession::PySessionInfo;
use crate::pysession::PyStreamingConfiguration;
use crate::pysession::{PyFireAndForgetConfiguration, PyRequestResponseConfiguration};
use crate::utils::PyAgentType;
use agp_config::grpc::client::ClientConfig as PyGrpcClientConfig;
use agp_config::grpc::server::ServerConfig as PyGrpcServerConfig;

// TODO(msardara): most of the structs here shouhld be generated with a macro
// to reflect any change that may occur in the gateway code

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
        self.sdk.agent.agent_id() as u64
    }
}

impl PyService {
    async fn create_pyservice_impl(
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
        let svc_id = agp_config::component::id::ID::new_with_str("service/0").unwrap();

        // create local service
        let svc = Service::new(svc_id);

        // Get the rx channel
        let rx = svc.create_agent(&agent).await?;

        // create the service
        let sdk = Arc::new(PyServiceInternal {
            service: svc,
            agent: agent,
            rx: RwLock::new(rx),
        });

        Ok(PyService { sdk: sdk })
    }

    async fn create_session_impl(
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

    async fn serve_impl(&self, config: PyGrpcServerConfig) -> Result<(), ServiceError> {
        self.sdk.service.run_server(&config)
    }

    async fn stop_impl(&self, endpoint: &str) -> Result<(), ServiceError> {
        self.sdk.service.stop_server(endpoint)
    }

    async fn connect_impl(&self, config: PyGrpcClientConfig) -> Result<u64, ServiceError> {
        // Get service and connect
        self.sdk.service.connect(&config).await
    }

    async fn disconnect_impl(&self, conn: u64) -> Result<(), ServiceError> {
        self.sdk.service.disconnect(conn)
    }

    async fn subscribe_impl(
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

    async fn unsubscribe_impl(
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

    async fn set_route_impl(
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

    async fn remove_route_impl(
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

    async fn publish_impl(
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
                        session_info.input_connection.clone(),
                    ),
                    None => {
                        return Err(ServiceError::ConfigError("no agent specified".to_string()));
                    }
                }
            }
        };

        // set flags
        let flags = AgpHeaderFlags::new(fanout, None, conn_out, None, None);

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

    async fn receive_impl(&self) -> Result<(PySessionInfo, Vec<u8>), ServiceError> {
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
                        agp_datapath::pubsub::ProtoPublishType(publish) => &publish.get_payload().blob,
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
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config=PyFireAndForgetConfiguration::default()))]
pub fn create_ff_session(
    py: Python,
    svc: PyService,
    config: PyFireAndForgetConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.create_session_impl(session::SessionConfig::FireAndForget(
            config.fire_and_forget_configuration,
        ))
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config=PyRequestResponseConfiguration::default()))]
pub fn create_rr_session(
    py: Python,
    svc: PyService,
    config: PyRequestResponseConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.create_session_impl(session::SessionConfig::RequestResponse(
            config.request_response_configuration,
        ))
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config))]
pub fn create_streaming_session(
    py: Python,
    svc: PyService,
    config: PyStreamingConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.create_session_impl(session::SessionConfig::Streaming(
            config.streaming_configuration,
        ))
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.serve_impl(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.stop_impl(&endpoint)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.connect_impl(config)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
pub fn disconnect(py: Python, svc: PyService, conn: u64) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        svc.disconnect_impl(conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.subscribe_impl(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.unsubscribe_impl(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.set_route_impl(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.remove_route_impl(conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
        svc.publish_impl(session_info.session_info, fanout, blob, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
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
            svc.receive_impl()
                .await
                .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
        },
    )
}

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
        PyService::create_pyservice_impl(organization, namespace, agent_type, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}
