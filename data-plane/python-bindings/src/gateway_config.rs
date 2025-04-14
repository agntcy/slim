// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pymethods;

use agp_config::auth::basic::Config as BasicAuthConfig;
use agp_config::grpc::{
    client::AuthenticationConfig as ClientAuthenticationConfig, client::ClientConfig,
    server::AuthenticationConfig as ServerAuthenticationConfig, server::ServerConfig,
};
use agp_config::tls::{client::TlsClientConfig, server::TlsServerConfig};
use agp_service::ServiceError;

/// gatewayconfig class
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub(crate) struct PyGatewayConfig {
    #[pyo3(get, set)]
    pub(crate) endpoint: String,

    #[pyo3(get, set)]
    pub(crate) insecure: bool,

    #[pyo3(get, set)]
    pub(crate) insecure_skip_verify: bool,

    #[pyo3(get, set)]
    pub(crate) tls_ca_path: Option<String>,

    #[pyo3(get, set)]
    pub(crate) tls_ca_pem: Option<String>,

    #[pyo3(get, set)]
    pub(crate) tls_cert_path: Option<String>,

    #[pyo3(get, set)]
    pub(crate) tls_key_path: Option<String>,

    #[pyo3(get, set)]
    pub(crate) tls_cert_pem: Option<String>,

    #[pyo3(get, set)]
    pub(crate) tls_key_pem: Option<String>,

    #[pyo3(get, set)]
    pub(crate) basic_auth_username: Option<String>,

    #[pyo3(get, set)]
    pub(crate) basic_auth_password: Option<String>,
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
    pub(crate) fn to_server_config(&self) -> Result<ServerConfig, ServiceError> {
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

    pub(crate) fn to_client_config(&self) -> Result<ClientConfig, ServiceError> {
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

#[derive(IntoPyObject)]
#[pyo3(transparent)]
struct PyBasicAuth {
    inner: BasicAuthConfig,
}
