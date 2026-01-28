// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_session::session_controller::SessionController;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionContext {
    pub session_id: u64,
    pub source_name: String,
    pub destination_name: String,
    pub metadata: HashMap<String, String>,
}

impl SessionContext {
    pub fn from_session(session: &Arc<SessionController>) -> Self {
        Self {
            session_id: session.id() as u64,
            source_name: session.source().to_string(),
            destination_name: session.dst().to_string(),
            metadata: session.metadata(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RpcMessageContext {
    pub source_name: String,
    pub destination_name: String,
    pub payload_type: String,
    pub metadata: HashMap<String, String>,
}

impl RpcMessageContext {
    pub fn from_message(msg: &slim_datapath::api::ProtoMessage) -> Self {
        Self {
            source_name: msg.get_slim_header().get_source().to_string(),
            destination_name: msg.get_slim_header().get_dst().to_string(),
            payload_type: "application".to_string(), // TODO: Get from message
            metadata: msg.get_metadata_map(),
        }
    }
}

// Keep MessageContext as alias for backward compatibility within slimrpc
pub type MessageContext = RpcMessageContext;

// Keep Context as alias for backward compatibility
pub type Context = SessionContext;
