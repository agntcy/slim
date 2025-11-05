// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports

// Third-party crates
use slim_datapath::api::ProtoMessage as Message;

use crate::context::SessionContext;

/// Session context
pub enum Notification {
    /// New session notification
    NewSession(SessionContext),
    /// Normal message notification
    NewMessage(Box<Message>),
}
