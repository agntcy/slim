// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoMessage as Message;

use crate::Transmitter;
use crate::context::SessionContext;
use crate::transmitter::SessionTransmitter;

/// Session context
pub enum Notification<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// New session notification
    NewSession(SessionContext<P, V, SessionTransmitter>),
    /// Normal message notification
    NewMessage(Box<Message>),
}
