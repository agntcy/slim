// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod gateway_config;
mod pysession;
mod utils;

use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use pysession::PyStreamingConfiguration;
use rand::Rng;
use tokio::sync::OnceCell;
use tokio::sync::RwLock;

use crate::pysession::{PyFireAndForgetConfiguration, PyRequestResponseConfiguration};
use agp_datapath::messages::encoder::{Agent, AgentType};
use agp_datapath::messages::utils::AgpHeaderFlags;
use agp_service::session;
use agp_service::{Service, ServiceError};
use gateway_config::PyGatewayConfig;
use pysession::{PySessionDirection, PySessionInfo};
use utils::PyAgentType;

static TRACING_GUARD: OnceCell<agp_tracing::OtelGuard> = OnceCell::const_new();

// TODO(msardara): most of the structs here shouhld be generated with a macro
// to reflect any change that may occur in the gateway code

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
struct PyService {
    sdk: Arc<PyServiceInternal>,
    config: Option<PyGatewayConfig>,
}

struct PyServiceInternal {
    service: Service,
    agent: Agent,
    rx: RwLock<session::AppChannelReceiver>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyService {
    #[pyo3(signature = (config))]
    pub fn configure(&mut self, config: PyGatewayConfig) {
        self.config = Some(config);
    }

    #[getter]
    pub fn id(&self) -> u64 {
        self.sdk.agent.agent_id() as u64
    }
}

async fn create_session_impl(
    svc: PyService,
    session_config: session::SessionConfig,
) -> Result<PySessionInfo, ServiceError> {
    Ok(PySessionInfo::from(
        svc.sdk
            .service
            .create_session(&svc.sdk.agent, session_config)
            .await?,
    ))
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config=PyFireAndForgetConfiguration::default()))]
fn create_ff_session(
    py: Python,
    svc: PyService,
    config: PyFireAndForgetConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        create_session_impl(
            svc.clone(),
            session::SessionConfig::FireAndForget(config.fire_and_forget_configuration),
        )
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config=PyRequestResponseConfiguration::default()))]
fn create_rr_session(
    py: Python,
    svc: PyService,
    config: PyRequestResponseConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        create_session_impl(
            svc.clone(),
            session::SessionConfig::RequestResponse(config.request_response_configuration),
        )
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, config))]
fn create_streaming_session(
    py: Python,
    svc: PyService,
    config: PyStreamingConfiguration,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        create_session_impl(
            svc.clone(),
            session::SessionConfig::Streaming(config.streaming_configuration),
        )
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn serve_impl(svc: PyService) -> Result<(), ServiceError> {
    let config = match svc.config {
        Some(config) => config,
        None => {
            return Err(ServiceError::ConfigError(
                "No configuration set on service".to_string(),
            ))
        }
    };

    let server_config = config.to_server_config()?;
    svc.sdk.service.serve(Some(server_config))
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc,
))]
fn serve(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        serve_impl(svc.clone())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn stop_impl(svc: PyService) -> Result<(), ServiceError> {
    Ok(svc.sdk.service.stop())
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc,
))]
fn stop(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        stop_impl(svc.clone())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn connect_impl(svc: PyService) -> Result<u64, ServiceError> {
    // Get the service's configuration
    let config = match svc.config {
        Some(config) => config,
        None => {
            return Err(ServiceError::ConfigError(
                "No configuration set on service".to_string(),
            ))
        }
    };

    // Convert PyGatewayConfig to ClientConfig
    let client_config = config.to_client_config()?;

    // Get service and connect
    svc.sdk.service.connect(Some(client_config)).await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (
    svc,
))]
fn connect(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        connect_impl(svc.clone())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn disconnect_impl(svc: PyService, conn: u64) -> Result<(), ServiceError> {
    svc.sdk.service.disconnect(conn)
}

#[gen_stub_pyfunction]
#[pyfunction]
fn disconnect(py: Python, svc: PyService, conn: u64) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        disconnect_impl(clone, conn)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn subscribe_impl(
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> Result<(), ServiceError> {
    let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);

    svc.sdk
        .service
        .subscribe(&svc.sdk.agent, &class, id, Some(conn))
        .await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
fn subscribe(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        subscribe_impl(clone, conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn unsubscribe_impl(
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> Result<(), ServiceError> {
    let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
    svc.sdk
        .service
        .unsubscribe(&svc.sdk.agent, &class, id, Some(conn))
        .await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
fn unsubscribe(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        unsubscribe_impl(clone, conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn set_route_impl(
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> Result<(), ServiceError> {
    let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
    svc.sdk
        .service
        .set_route(&svc.sdk.agent, &class, id, conn)
        .await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
fn set_route(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        set_route_impl(clone, conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn remove_route_impl(
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> Result<(), ServiceError> {
    let class = AgentType::from_strings(&name.organization, &name.namespace, &name.agent_type);
    svc.sdk
        .service
        .remove_route(&svc.sdk.agent, &class, id, conn)
        .await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, conn, name, id=None))]
fn remove_route(
    py: Python,
    svc: PyService,
    conn: u64,
    name: PyAgentType,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        remove_route_impl(clone, conn, name, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn publish_impl(
    svc: PyService,
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
                None => return Err(ServiceError::ConfigError("no agent specified".to_string())),
            }
        }
    };

    // set flags
    let flags = AgpHeaderFlags::new(fanout, None, conn_out, None, None);

    svc.sdk
        .service
        .publish_with_flags(
            &svc.sdk.agent,
            session_info,
            &agent_type,
            agent_id,
            flags,
            blob,
        )
        .await
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, session_info, fanout, blob, name=None, id=None))]
fn publish(
    py: Python,
    svc: PyService,
    session_info: PySessionInfo,
    fanout: u32,
    blob: Vec<u8>,
    name: Option<PyAgentType>,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        publish_impl(
            svc.clone(),
            session_info.session_info,
            fanout,
            blob,
            name,
            id,
        )
        .await
        .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn receive_impl(svc: PyService) -> Result<(PySessionInfo, Vec<u8>), ServiceError> {
    let mut rx = svc.sdk.rx.write().await;

    let msg = rx
        .recv()
        .await
        .ok_or(ServiceError::ConfigError("no message received".to_string()))?
        .map_err(|e| ServiceError::ReceiveError(e.to_string()))?;

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

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc))]
fn receive(py: Python, svc: PyService) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py_with_locals(
        py,
        pyo3_async_runtimes::tokio::get_current_locals(py)?,
        async move {
            receive_impl(svc.clone())
                .await
                .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
        },
    )
}

async fn init_tracing_impl(log_level: String, enable_opentelemetry: bool) {
    let _ = TRACING_GUARD
        .get_or_init(|| async {
            let mut config = agp_tracing::TracingConfiguration::default().with_log_level(log_level);

            if enable_opentelemetry {
                config = config.clone().enable_opentelemetry();
            }

            let otel_guard = config.setup_tracing_subscriber();

            otel_guard
        })
        .await;
}

#[pyfunction]
#[pyo3(signature = (log_level="info".to_string(), enable_opentelemetry=false,))]
fn init_tracing(py: Python, log_level: String, enable_opentelemetry: bool) {
    let _ = pyo3_async_runtimes::tokio::future_into_py(py, async move {
        Ok(init_tracing_impl(log_level, enable_opentelemetry).await)
    });
}

async fn create_pyservice_impl(
    organization: String,
    namespace: String,
    agent_type: String,
    id: Option<u64>,
) -> Result<PyService, ServiceError> {
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

    Ok(PyService {
        sdk: sdk,
        config: None,
    })
}

#[pyfunction]
#[pyo3(signature = (organization, namespace, agent_type, id=None))]
fn create_pyservice(
    py: Python,
    organization: String,
    namespace: String,
    agent_type: String,
    id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        create_pyservice_impl(organization, namespace, agent_type, id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

#[pymodule]
fn _agp_bindings(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyGatewayConfig>()?;
    m.add_class::<PyService>()?;
    m.add_class::<PyAgentType>()?;
    m.add_class::<PySessionInfo>()?;
    m.add_class::<PyFireAndForgetConfiguration>()?;
    m.add_class::<PyRequestResponseConfiguration>()?;
    m.add_class::<PyStreamingConfiguration>()?;
    m.add_class::<PySessionDirection>()?;

    m.add_function(wrap_pyfunction!(create_pyservice, m)?)?;
    m.add_function(wrap_pyfunction!(create_ff_session, m)?)?;
    m.add_function(wrap_pyfunction!(create_rr_session, m)?)?;
    m.add_function(wrap_pyfunction!(create_streaming_session, m)?)?;
    m.add_function(wrap_pyfunction!(subscribe, m)?)?;
    m.add_function(wrap_pyfunction!(unsubscribe, m)?)?;
    m.add_function(wrap_pyfunction!(set_route, m)?)?;
    m.add_function(wrap_pyfunction!(remove_route, m)?)?;
    m.add_function(wrap_pyfunction!(publish, m)?)?;
    m.add_function(wrap_pyfunction!(serve, m)?)?;
    m.add_function(wrap_pyfunction!(stop, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;
    m.add_function(wrap_pyfunction!(disconnect, m)?)?;
    m.add_function(wrap_pyfunction!(receive, m)?)?;
    m.add_function(wrap_pyfunction!(init_tracing, m)?)?;

    m.add("SESSION_UNSPECIFIED", session::SESSION_UNSPECIFIED)?;

    Ok(())
}

// Define a function to gather stub information.
define_stub_info_gatherer!(stub_info);
