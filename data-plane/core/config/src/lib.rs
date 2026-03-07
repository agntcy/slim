// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod auth;
pub mod backoff;
pub mod client;
pub mod component;
pub mod grpc;
pub mod provider;
pub mod server;
pub mod testutils;
pub mod tls;
pub mod transport;
pub mod websocket;

mod opaque;

pub const CLIENT_CONFIG_SCHEMA_JSON: &str = include_str!("./schema/client-config.schema.json");
pub const SERVER_CONFIG_SCHEMA_JSON: &str = include_str!("./schema/server-config.schema.json");
