// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyclass;

pub(crate) use agp_config::grpc::client::ClientConfig as PyGrpcClientConfig;
pub(crate) use agp_config::grpc::server::ServerConfig as PyGrpcServerConfig;

// use agp_config::grpc::{
//     client::ClientConfig as GrpcClientConfig,
//     server::ServerConfig as GrpcServerConfig,
// };

// #[gen_stub_pyclass]
// #[derive(FromPyObject)]
// #[pyclass]
// pub struct PyGrpcClientConfig(pub GrpcClientConfig);

// #[gen_stub_pyclass]
// #[derive(FromPyObject)]
// #[pyclass]
// pub struct PyGrpcServerConfig(pub GrpcServerConfig);
