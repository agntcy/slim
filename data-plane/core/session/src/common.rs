// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tonic::Status;

use slim_datapath::{
    api::{
        ApplicationPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType,
        SessionHeader, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::SessionError;

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

#[allow(clippy::too_many_arguments)]
pub fn new_message_from_session_fields(
    local_name: &Name,
    target_name: &Name,
    target_conn: u64,
    is_error: bool,
    session_type: ProtoSessionType,
    message_type: ProtoSessionMessageType,
    session_id: u32,
    message_id: u32,
) -> Message {
    let flags = if is_error {
        Some(
            SlimHeaderFlags::default()
                .with_forward_to(target_conn)
                .with_error(true),
        )
    } else {
        Some(SlimHeaderFlags::default().with_forward_to(target_conn))
    };

    let slim_header = Some(SlimHeader::new(local_name, target_name, "", flags));

    let session_header = Some(SessionHeader::new(
        session_type.into(),
        message_type.into(),
        session_id,
        message_id,
    ));

    Message::new_publish_with_headers(
        slim_header,
        session_header,
        Some(ApplicationPayload::new("", vec![]).as_content()),
    )
}

/// Message types for communication between session components
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum SessionMessage {
    /// application message coming from the app or from slim
    OnMessage {
        message: Message,
        direction: MessageDirection,
    },
    /// timeout signal for a message (ack,rtx or control messages)
    /// that needs to be send again
    TimerTimeout {
        message_id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
        timeouts: u32,
    },
    /// timer failure, signal to the owner of the packet that
    /// the message will not be delivered
    TimerFailure {
        message_id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
        timeouts: u32,
    },
    /// message from session controller to session layer
    /// to notify that the session can be removed safely
    DeleteSession { session_id: u32 },
    /// message from session layer to the session controller
    /// to start to the close procedures of the session
    StartDrain { grace_period_ms: u64 },
}
