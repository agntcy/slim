// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pyfunction;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use rand::Rng;
use tokio::sync::OnceCell;
use tokio::sync::RwLock;

use agp_config::auth::basic::Config as BasicAuthConfig;
use agp_config::grpc::{
    client::AuthenticationConfig as ClientAuthenticationConfig, client::ClientConfig,
    server::AuthenticationConfig as ServerAuthenticationConfig, server::ServerConfig,
};
use agp_config::tls::{client::TlsClientConfig, server::TlsServerConfig};
use agp_datapath::messages::encoder::{Agent, AgentType};
use agp_datapath::messages::utils::AgpHeaderFlags;
use agp_service::session;
use agp_service::{Service, ServiceError};

static TRACING_GUARD: OnceCell<agp_tracing::OtelGuard> = OnceCell::const_new();

// TODO(msardara): most of the structs here shouhld be generated with a macro
// to reflect any change that may occur in the gateway code

/// gatewayconfig class
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
struct PyGatewayConfig {
    #[pyo3(get, set)]
    endpoint: String,

    #[pyo3(get, set)]
    insecure: bool,

    #[pyo3(get, set)]
    insecure_skip_verify: bool,

    #[pyo3(get, set)]
    tls_ca_path: Option<String>,

    #[pyo3(get, set)]
    tls_ca_pem: Option<String>,

    #[pyo3(get, set)]
    tls_cert_path: Option<String>,

    #[pyo3(get, set)]
    tls_key_path: Option<String>,

    #[pyo3(get, set)]
    tls_cert_pem: Option<String>,

    #[pyo3(get, set)]
    tls_key_pem: Option<String>,

    #[pyo3(get, set)]
    basic_auth_username: Option<String>,

    #[pyo3(get, set)]
    basic_auth_password: Option<String>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyGatewayConfig {
    #[new]
    #[pyo3(signature = (
        endpoint,
        insecure=false,
        insecure_skip_verify=false,
        tls_ca_path=None,
        tls_ca_pem=None,
        tls_cert_path=None,
        tls_key_path=None,
        tls_cert_pem=None,
        tls_key_pem=None,
        basic_auth_username=None,
        basic_auth_password=None,
    ))]
    pub fn new(
        endpoint: String,
        insecure: bool,
        insecure_skip_verify: bool,
        tls_ca_path: Option<String>,
        tls_ca_pem: Option<String>,
        tls_cert_path: Option<String>,
        tls_key_path: Option<String>,
        tls_cert_pem: Option<String>,
        tls_key_pem: Option<String>,
        basic_auth_username: Option<String>,
        basic_auth_password: Option<String>,
    ) -> Self {
        PyGatewayConfig {
            endpoint,
            insecure,
            insecure_skip_verify,
            tls_ca_path,
            tls_ca_pem,
            tls_cert_path,
            tls_key_path,
            tls_cert_pem,
            tls_key_pem,
            basic_auth_username,
            basic_auth_password,
        }
    }
}

impl PyGatewayConfig {
    fn to_server_config(&self) -> Result<ServerConfig, ServiceError> {
        let config = ServerConfig::with_endpoint(&self.endpoint);
        let tls_settings = TlsServerConfig::new().with_insecure(self.insecure);
        let tls_settings = match (&self.tls_cert_path, &self.tls_key_path) {
            (Some(cert_path), Some(key_path)) => tls_settings
                .with_cert_file(cert_path)
                .with_key_file(key_path),
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use server cert without key".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use server key without cert".to_string(),
                ));
            }
            (_, _) => tls_settings,
        };

        let tls_settings = match (&self.tls_cert_pem, &self.tls_key_pem) {
            (Some(cert_pem), Some(key_pem)) => {
                tls_settings.with_cert_pem(cert_pem).with_key_pem(key_pem)
            }
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use server cert PEM without key PEM".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use server key PEM without cert PEM".to_string(),
                ));
            }
            (_, _) => tls_settings,
        };

        let config = config.with_tls_settings(tls_settings);

        let config = match (&self.basic_auth_username, &self.basic_auth_password) {
            (Some(username), Some(password)) => config.with_auth(
                ServerAuthenticationConfig::Basic(BasicAuthConfig::new(username, password)),
            ),
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use basic auth without password".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use basic auth without username".to_string(),
                ));
            }
            (_, _) => config,
        };

        Ok(config)
    }

    fn to_client_config(&self) -> Result<ClientConfig, ServiceError> {
        let config = ClientConfig::with_endpoint(&self.endpoint);

        let tls_settings = TlsClientConfig::new()
            .with_insecure(self.insecure)
            .with_insecure_skip_verify(self.insecure_skip_verify);

        let tls_settings = match &self.tls_ca_path {
            Some(ca_path) => tls_settings.with_ca_file(ca_path),
            None => tls_settings,
        };

        let tls_settings = match &self.tls_ca_pem {
            Some(ca_pem) => tls_settings.with_ca_pem(ca_pem),
            None => tls_settings,
        };

        let tls_settings = match (&self.tls_cert_path, &self.tls_key_path) {
            (Some(cert_path), Some(key_path)) => tls_settings
                .with_cert_file(cert_path)
                .with_key_file(key_path),
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use client cert without key".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use client key without cert".to_string(),
                ));
            }
            (_, _) => tls_settings,
        };

        let tls_settings = match (&self.tls_cert_pem, &self.tls_key_pem) {
            (Some(cert_pem), Some(key_pem)) => {
                tls_settings.with_cert_pem(cert_pem).with_key_pem(key_pem)
            }
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use client cert PEM without key PEM".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use client key PEM without cert PEM".to_string(),
                ));
            }
            (_, _) => tls_settings,
        };

        let config = config.with_tls_setting(tls_settings);

        let config = match (&self.basic_auth_username, &self.basic_auth_password) {
            (Some(username), Some(password)) => config.with_auth(
                ClientAuthenticationConfig::Basic(BasicAuthConfig::new(username, password)),
            ),
            (Some(_), None) => {
                return Err(ServiceError::ConfigError(
                    "cannot use basic auth without password".to_string(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServiceError::ConfigError(
                    "cannot use basic auth without username".to_string(),
                ));
            }
            (_, _) => config,
        };

        Ok(config)
    }
}

/// agent class
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
struct PyAgentType {
    organization: String,
    namespace: String,
    agent_type: String,
}

impl Into<AgentType> for PyAgentType {
    fn into(self) -> AgentType {
        AgentType::from_strings(&self.organization, &self.namespace, &self.agent_type)
    }
}

impl Into<AgentType> for &PyAgentType {
    fn into(self) -> AgentType {
        AgentType::from_strings(&self.organization, &self.namespace, &self.agent_type)
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyAgentType {
    #[new]
    pub fn new(agent_org: String, agent_ns: String, agent_class: String) -> Self {
        PyAgentType {
            organization: agent_org,
            namespace: agent_ns,
            agent_type: agent_class,
        }
    }
}

/// session config
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone, Default)]
struct PyFireAndForgetConfiguration {
    pub fire_and_forget_configuration: agp_service::FireAndForgetConfiguration,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyFireAndForgetConfiguration {
    #[new]
    pub fn new() -> Self {
        PyFireAndForgetConfiguration {
            fire_and_forget_configuration: agp_service::FireAndForgetConfiguration {},
        }
    }
}

/// session config
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone, Default)]
struct PyRequestResponseConfiguration {
    pub request_response_configuration: agp_service::RequestResponseConfiguration,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyRequestResponseConfiguration {
    #[new]
    #[pyo3(signature = (max_retries=0, timeout=1000))]
    pub fn new(max_retries: u32, timeout: u32) -> Self {
        PyRequestResponseConfiguration {
            request_response_configuration: agp_service::RequestResponseConfiguration {
                max_retries,
                timeout: std::time::Duration::from_millis(timeout as u64),
            },
        }
    }

    #[getter]
    pub fn max_retries(&self) -> u32 {
        self.request_response_configuration.max_retries
    }

    #[getter]
    pub fn timeout(&self) -> u32 {
        self.request_response_configuration.timeout.as_millis() as u32
    }

    #[setter]
    pub fn set_max_retries(&mut self, max_retries: u32) {
        self.request_response_configuration.max_retries = max_retries;
    }

    #[setter]
    pub fn set_timeout(&mut self, timeout: u32) {
        self.request_response_configuration.timeout =
            std::time::Duration::from_millis(timeout as u64);
    }
}

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
struct PySessionInfo {
    session_info: session::Info,
}

impl From<session::Info> for PySessionInfo {
    fn from(session_info: session::Info) -> Self {
        PySessionInfo { session_info }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PySessionInfo {
    #[new]
    fn new(session_id: u32) -> Self {
        PySessionInfo {
            session_info: session::Info::new(session_id),
        }
    }

    #[getter]
    fn id(&self) -> u32 {
        self.session_info.id
    }
}

#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
struct PyService {
    sdk: Arc<tokio::sync::RwLock<PyServiceInternal>>,
    config: Option<PyGatewayConfig>,
}

struct PyServiceInternal {
    service: Service,
    agent: Option<Agent>,
    rx: Option<session::AppChannelReceiver>,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyService {
    #[new]
    pub fn new(id: &str) -> Self {
        let svc_id = agp_config::component::id::ID::new_with_str(id).unwrap();
        PyService {
            sdk: Arc::new(RwLock::new(PyServiceInternal {
                service: Service::new(svc_id),
                agent: None,
                rx: None,
            })),
            config: None,
        }
    }

    #[pyo3(signature = (config))]
    pub fn configure(&mut self, config: PyGatewayConfig) {
        self.config = Some(config);
    }
}

async fn create_agent_impl(
    svc: PyService,
    agent_org: String,
    agent_ns: String,
    agent_class: String,
    agent_id: Option<u64>,
) -> Result<u64, ServiceError> {
    let id = match agent_id {
        Some(v) => v,
        None => {
            let mut rng = rand::rng();
            rng.random()
        }
    };

    // create local agent
    let agent = Agent::from_strings(&agent_org, &agent_ns, &agent_class, id);
    let mut service = svc.sdk.write().await;
    let rx = service.service.create_agent(&agent)?;
    service.rx = Some(rx);
    service.agent = Some(agent);

    Ok(id)
}

#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (svc, agent_org, agent_ns, agent_class, agent_id=None))]
fn create_agent(
    py: Python,
    svc: PyService,
    agent_org: String,
    agent_ns: String,
    agent_class: String,
    agent_id: Option<u64>,
) -> PyResult<Bound<PyAny>> {
    let clone = svc.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        create_agent_impl(clone, agent_org, agent_ns, agent_class, agent_id)
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn create_session_impl(
    svc: PyService,
    session_config: session::SessionConfig,
) -> Result<PySessionInfo, ServiceError> {
    // create local agent
    let service = svc.sdk.write().await;

    match &service.agent {
        Some(agent) => {
            let session_info = service
                .service
                .create_session(agent, session_config)
                .await?;
            Ok(PySessionInfo::from(session_info))
        }
        None => Err(ServiceError::AgentNotFound("no agent found".to_string())),
    }
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

    let service = svc.sdk.write().await;
    service.service.serve(Some(server_config))
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
    let service = svc.sdk.write().await;
    service.service.stop();

    Ok(())
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
    let mut service = svc.sdk.write().await;
    service.service.connect(Some(client_config)).await
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
    let mut service = svc.sdk.write().await;
    service.service.disconnect(conn)
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
    let service = svc.sdk.read().await;

    match &service.agent {
        Some(agent) => {
            service
                .service
                .subscribe(agent, &class, id, Some(conn))
                .await
        }
        None => Err(ServiceError::AgentNotFound(
            "missing from service".to_string(),
        )),
    }
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
    let service = svc.sdk.read().await;

    match &service.agent {
        Some(agent) => {
            service
                .service
                .unsubscribe(agent, &class, id, Some(conn))
                .await
        }
        None => Err(ServiceError::AgentNotFound(
            "missing from service".to_string(),
        )),
    }
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
    let service = svc.sdk.read().await;

    match &service.agent {
        Some(agent) => service.service.set_route(agent, &class, id, conn).await,
        None => Err(ServiceError::AgentNotFound(
            "missing from service".to_string(),
        )),
    }
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
    let service = svc.sdk.read().await;

    match &service.agent {
        Some(agent) => service.service.remove_route(agent, &class, id, conn).await,
        None => Err(ServiceError::AgentNotFound(
            "missing from service".to_string(),
        )),
    }
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
                Some(ref agent) => (
                    agent.agent_type().clone(),
                    Some(agent.agent_id()),
                    session_info.input_connection.clone(),
                ),
                None => return Err(ServiceError::ConfigError("no agent specified".to_string())),
            }
        }
    };

    let service = svc.sdk.read().await;

    // set flags
    let flags = AgpHeaderFlags::new(fanout, None, conn_out, None, None);
    match &service.agent {
        Some(agent) => {
            service
                .service
                .publish_with_flags(agent, session_info, &agent_type, agent_id, flags, blob)
                .await
        }
        None => Err(ServiceError::AgentNotFound(
            "missing from service".to_string(),
        )),
    }
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
    let mut service = svc.sdk.write().await;

    let rx = service.rx.as_mut().ok_or(ServiceError::ReceiveError(
        "no local agent created".to_string(),
    ))?;

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
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        receive_impl(svc.clone())
            .await
            .map_err(|e| PyErr::new::<PyException, _>(format!("{}", e.to_string())))
    })
}

async fn init_tracing_impl(
    log_level: String,
    enable_opentelemetry: bool,
) {
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
fn init_tracing(
    py: Python,
    log_level: String,
    enable_opentelemetry: bool,
) {
    let _ = pyo3_async_runtimes::tokio::future_into_py(py, async move {
        Ok(init_tracing_impl(log_level, enable_opentelemetry).await)
    });
}

#[pymodule]
fn _agp_bindings(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyGatewayConfig>()?;
    m.add_class::<PyService>()?;
    m.add_class::<PyAgentType>()?;
    m.add_class::<PySessionInfo>()?;
    m.add_class::<PyFireAndForgetConfiguration>()?;
    m.add_class::<PyRequestResponseConfiguration>()?;

    m.add_function(wrap_pyfunction!(create_agent, m)?)?;
    m.add_function(wrap_pyfunction!(create_ff_session, m)?)?;
    m.add_function(wrap_pyfunction!(create_rr_session, m)?)?;
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

    Ok(())
}

// Define a function to gather stub information.
define_stub_info_gatherer!(stub_info);
