// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod auth;
pub mod component;
pub mod grpc;
pub mod provider;
pub mod testutils;
pub mod tls;

mod opaque;

pub const CLIENT_CONFIG_SCHEMA_JSON: &str = include_str!("./grpc/schema/client-config.schema.json");
