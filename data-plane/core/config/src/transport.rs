// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Transport protocol used by client and server dataplane configuration.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    /// gRPC transport (default).
    #[default]
    Grpc,
    /// Native websocket transport.
    Websocket,
}
