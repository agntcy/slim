// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::messages::Name;
use slim_session::session_controller::SessionController;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: u64,
    pub source_name: Name,
    pub destination_name: Name,
    pub metadata: Option<HashMap<String, String>>,
}

impl SessionContext {
    pub fn from_session(session: &Arc<SessionController>) -> Self {
        Self {
            session_id: session.id() as u64,
            source_name: session.source().clone(),
            destination_name: session.dst().clone(),
            metadata: Some(session.metadata()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageContext {
    pub source_name: Name,
    pub destination_name: Name,
    pub payload_type: String,
    pub metadata: Option<HashMap<String, String>>,
}

impl MessageContext {
    pub fn from_message(msg: &slim_datapath::api::ProtoMessage) -> Self {
        Self {
            source_name: msg.get_slim_header().get_source(),
            destination_name: msg.get_slim_header().get_dst(),
            payload_type: "application".to_string(), // TODO: Get from message
            metadata: Some(msg.get_metadata_map()),
        }
    }
}

// Keep Context as alias for backward compatibility
pub type Context = SessionContext;
