// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

// Third-party crates
use tonic::Status;

use slim_datapath::{
    api::{EncodedName, ProtoMessage as Message, ProtoSessionMessageType},
    messages::Name,
};

// Local crate
use crate::SessionError;

/// Reserved session id
pub const SESSION_RANGE: std::ops::Range<u32> = 0..(u32::MAX - 1000);

/// Unspecified session ID constant
pub const SESSION_UNSPECIFIED: u32 = u32::MAX;

/// Channel used in the path service -> app
pub(crate) type AppChannelSender =
    tokio::sync::mpsc::UnboundedSender<Result<Message, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::UnboundedReceiver<Result<Message, SessionError>>;
/// Channel used in the path service -> slim
pub type SlimChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
#[allow(dead_code)]
pub enum State {
    Active,
    Inactive,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MessageDirection {
    North,
    South,
}

/// Message types for communication between session components
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum SessionMessage {
    /// application message coming from the app or from slim
    OnMessage {
        message: Message,
        direction: MessageDirection,
        /// Optional channel to signal when message processing is complete
        ack_tx: Option<tokio::sync::oneshot::Sender<Result<(), SessionError>>>,
    },
    /// Error occurred during message processing
    MessageError { error: SessionError },
    /// timeout signal for a message (ack,rtx or control messages)
    /// that needs to be send again
    TimerTimeout {
        message_id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<EncodedName>,
        timeouts: u32,
    },
    /// timer failure, signal to the owner of the packet that
    /// the message will not be delivered
    TimerFailure {
        message_id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<EncodedName>,
        timeouts: u32,
    },
    /// sent by the controller sender when a disconnection is detected
    ParticipantDisconnected { name: Option<Name> },
    /// message from session layer to the session controller
    /// to start to the close procedures of the session
    StartDrain { grace_period: Duration },
    /// message from session controller to session layer
    /// to notify that the session can be removed safely
    DeleteSession { session_id: u32 },
    /// Query the participants list from the handler
    GetParticipantsList {
        tx: tokio::sync::oneshot::Sender<Vec<Name>>,
    },
}
