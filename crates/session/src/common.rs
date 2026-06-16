// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

// Third-party crates
use smallvec::SmallVec;
use tonic::Status;

use slim_datapath::api::{
    EncodedName, ProtoMessage as Message, ProtoName, ProtoSessionMessageType,
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

/// A message to be sent outbound from the session layers.
#[derive(Debug)]
pub enum OutboundMessage {
    /// Send to SLIM (network-bound). Identity will be applied by the processing loop.
    ToSlim(Message),
    /// Send to the application.
    ToApp(Result<Message, SessionError>),
}

/// The result of processing a session message.
/// Layers return this instead of sending messages internally.
/// Uses SmallVec since most operations produce 1-2 outbound messages.
#[derive(Debug, Default)]
pub struct SessionOutput {
    pub messages: SmallVec<[OutboundMessage; 2]>,
}

impl SessionOutput {
    pub fn new() -> Self {
        Self {
            messages: SmallVec::new(),
        }
    }

    /// Create a new SessionOutput containing a single ToSlim message.
    pub fn to_slim(message: Message) -> Self {
        let mut s = Self::new();
        s.messages.push(OutboundMessage::ToSlim(message));
        s
    }

    /// Create a new SessionOutput containing a single ToApp message.
    pub fn to_app(message: Result<Message, SessionError>) -> Self {
        let mut s = Self::new();
        s.messages.push(OutboundMessage::ToApp(message));
        s
    }

    /// Push a ToSlim message onto an existing output.
    pub fn push_slim(&mut self, message: Message) {
        self.messages.push(OutboundMessage::ToSlim(message));
    }

    /// Push a ToApp message onto an existing output.
    pub fn push_app(&mut self, message: Result<Message, SessionError>) {
        self.messages.push(OutboundMessage::ToApp(message));
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Merge another output into this one.
    pub fn extend(&mut self, other: SessionOutput) {
        self.messages.extend(other.messages);
    }
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
    ParticipantDisconnected { name: Option<ProtoName> },
    /// message from session layer to the session controller
    /// to start to the close procedures of the session
    StartDrain { grace_period: Duration },
    /// message from session controller to session layer
    /// to notify that the session can be removed safely
    DeleteSession { session_id: u32 },
    /// Query the participants list from the handler
    GetParticipantsList {
        tx: tokio::sync::oneshot::Sender<Vec<ProtoName>>,
    },
    /// Deferred cleanup after leave reply has been dispatched.
    /// Performs route/subscription cleanup that must happen after the LeaveReply
    /// is sent (in the return-based output model, dispatch happens after on_message returns).
    LeaveCleanup,
}
