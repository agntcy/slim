// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_config::transport::TransportProtocol as CoreTransportProtocol;

/// Transport protocol for dataplane communication.
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum TransportProtocol {
    Grpc,
    Websocket,
}

impl From<TransportProtocol> for CoreTransportProtocol {
    fn from(transport: TransportProtocol) -> Self {
        match transport {
            TransportProtocol::Grpc => CoreTransportProtocol::Grpc,
            TransportProtocol::Websocket => CoreTransportProtocol::Websocket,
        }
    }
}

impl From<CoreTransportProtocol> for TransportProtocol {
    fn from(transport: CoreTransportProtocol) -> Self {
        match transport {
            CoreTransportProtocol::Grpc => TransportProtocol::Grpc,
            CoreTransportProtocol::Websocket => TransportProtocol::Websocket,
        }
    }
}
