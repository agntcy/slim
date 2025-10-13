// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tonic::Status;

use slim_datapath::{api::ProtoMessage as Message, messages::Name};

// Local crate
use crate::{PointToPointConfiguration, SessionError};

/// Reserved session id
pub const SESSION_RANGE: std::ops::Range<u32> = 0..(u32::MAX - 1000);

/// Unspecified session ID constant
pub const SESSION_UNSPECIFIED: u32 = u32::MAX;

/// Channel used in the path service -> app
pub(crate) type AppChannelSender = tokio::sync::mpsc::Sender<Result<Message, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::Receiver<Result<Message, SessionError>>;
/// Channel used in the path service -> slim
pub type SlimChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
#[allow(dead_code)]
pub enum State {
    Active,
    Inactive,
}

#[derive(Clone, PartialEq, Debug)]
pub enum MessageDirection {
    North,
    South,
}

/// Message types for communication between session components
#[allow(clippy::large_enum_variant)]
pub enum SessionMessage {
    OnMessage {
        message: Message,
        direction: MessageDirection,
    },
    SetPointToPointConfig {
        config: PointToPointConfiguration,
    },
    TimerTimeout {
        message_id: u32,
        timeouts: u32,
        name: Option<Name>,
    },
    TimerFailure {
        message_id: u32,
        timeouts: u32,
        name: Option<Name>,
    },
    DeleteSession {
        session_id: u32,
    },
    AddEndpoint {
        endpoint: Name,
    },
    RemoveEndpoint {
        endpoint: Name,
    },
    Drain {
        grace_period_ms: u64,
    },
}
