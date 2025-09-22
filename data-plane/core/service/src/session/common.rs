// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tonic::Status;

use slim_datapath::api::ProtoMessage as Message;

// Local crate
use crate::session::SessionError;

/// Reserved session id
pub const SESSION_RANGE: std::ops::Range<u32> = 0..(u32::MAX - 1000);

/// Unspecified session ID constant
pub const SESSION_UNSPECIFIED: u32 = u32::MAX;

/// Channel used in the path service -> app
pub(crate) type AppChannelSender = tokio::sync::mpsc::Sender<Result<Message, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::Receiver<Result<Message, SessionError>>;
/// Channel used in the path service -> slim
pub(crate) type SlimChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
#[allow(dead_code)]
pub enum State {
    Active,
    Inactive,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum MessageDirection {
    North,
    South,
}

/// The session type
#[derive(Clone, PartialEq, Debug)]
pub enum SessionType {
    PointToPoint,
    Multicast,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::PointToPoint => write!(f, "PointToPoint"),
            SessionType::Multicast => write!(f, "Multicast"),
        }
    }
}

// Session implementation moved to handle.rs
