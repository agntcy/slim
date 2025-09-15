// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;

// Third-party crates
use slim_datapath::api::ProtoSessionType;
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType};
use slim_datapath::messages::encoder::Name;

/// Session ID
pub type Id = u32;

/// Session Info
#[derive(Clone, PartialEq, Debug)]
pub struct Info {
    /// The id of the session
    pub id: Id,
    /// The message nonce used to identify the message
    pub message_id: Option<u32>,
    /// The Message Type
    pub session_message_type: ProtoSessionMessageType,
    // The session Type
    pub session_type: ProtoSessionType,
    /// The identifier of the app that sent the message
    pub message_source: Option<Name>,
    /// The destination name of the message
    pub message_destination: Option<Name>,
    /// The input connection id
    pub input_connection: Option<u64>,
    /// The pyaload type in the packet
    pub payload_type: Option<String>,
    /// The metadata associated to the packet
    pub metadata: HashMap<String, String>,
}

impl Info {
    /// Create a new session info
    pub fn new(id: Id) -> Self {
        Info {
            id,
            message_id: None,
            session_message_type: ProtoSessionMessageType::Unspecified,
            session_type: ProtoSessionType::SessionUnknown,
            message_source: None,
            message_destination: None,
            input_connection: None,
            payload_type: None,
            metadata: HashMap::new(),
        }
    }

    pub fn set_message_id(&mut self, message_id: u32) {
        self.message_id = Some(message_id);
    }

    pub fn set_session_message_type(&mut self, session_header_type: ProtoSessionMessageType) {
        self.session_message_type = session_header_type;
    }

    pub fn set_session_type(&mut self, session_type: ProtoSessionType) {
        self.session_type = session_type;
    }

    pub fn set_message_source(&mut self, message_source: Name) {
        self.message_source = Some(message_source);
    }

    pub fn set_message_destination(&mut self, message_destination: Name) {
        self.message_destination = Some(message_destination);
    }

    pub fn set_input_connection(&mut self, input_connection: u64) {
        self.input_connection = Some(input_connection);
    }

    pub fn get_message_id(&self) -> Option<u32> {
        self.message_id
    }

    pub fn session_message_type_unset(&self) -> bool {
        self.session_message_type == ProtoSessionMessageType::Unspecified
    }

    pub fn get_session_message_type(&self) -> ProtoSessionMessageType {
        self.session_message_type
    }

    pub fn session_type_unset(&self) -> bool {
        self.session_type == ProtoSessionType::SessionUnknown
    }

    pub fn get_session_type(&self) -> ProtoSessionType {
        self.session_type
    }

    pub fn get_message_source(&self) -> Option<Name> {
        self.message_source.clone()
    }

    pub fn get_message_destination(&self) -> Option<Name> {
        self.message_destination.clone()
    }

    pub fn get_input_connection(&self) -> Option<u64> {
        self.input_connection
    }

    pub fn get_payload_type(&self) -> Option<String> {
        self.payload_type.clone()
    }

    pub fn get_metadata(&self) -> HashMap<String, String> {
        self.metadata.clone()
    }
}

impl From<&Message> for Info {
    fn from(message: &Message) -> Self {
        let session_header = message.get_session_header();
        let slim_header = message.get_slim_header();

        let id = session_header.session_id;
        let message_id = session_header.message_id;
        let message_source = message.get_source();
        let message_destination = message.get_dst();
        let input_connection = slim_header.incoming_conn;
        let session_message_type = session_header.session_message_type();
        let session_type = session_header.session_type();
        let payload_type = message.get_payload().map(|c| c.content_type.clone());
        let metadata = message.get_metadata_map();

        Info {
            id,
            message_id: Some(message_id),
            session_message_type,
            session_type,
            message_source: Some(message_source),
            message_destination: Some(message_destination),
            input_connection,
            payload_type,
            metadata,
        }
    }
}
