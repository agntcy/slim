// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;
use pyo3_stub_gen::derive::gen_stub_pymethods;
use pyo3::IntoPyObject;

use agp_config::auth::basic::Config as BasicAuthConfig;
use agp_config::auth::bearer::Config as BearerAuthConfig;
use agp_config::grpc::{
    client::AuthenticationConfig as ClientAuthenticationConfig, client::ClientConfig as GrpcClientConfig,
    server::AuthenticationConfig as ServerAuthenticationConfig, server::ServerConfig as GrpcServerConfig,
};
use agp_config::tls::{client::TlsClientConfig, server::TlsServerConfig};
use agp_service::ServiceError;

#[derive(IntoPyObject)]
struct PyBasicAuth(BasicAuthConfig);

#[derive(IntoPyObject)]
struct PyBearerAuth(BearerAuthConfig);

#[derive(IntoPyObject)]
struct PyClientAuthConfig(ClientAuthenticationConfig);

#[derive(IntoPyObject)]
struct PyServerAuthConfig(ServerAuthenticationConfig);

#[derive(IntoPyObject)]
struct PyGrpcClientConfig(GrpcClientConfig);

#[derive(IntoPyObject)]
struct PyGrpcServerConfig(GrpcServerConfig);
