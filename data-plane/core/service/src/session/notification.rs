// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoMessage as Message;

use crate::session::Transmitter;
use crate::session::context::SessionContext;
use crate::session::transmitter::SessionTransmitter;

/// Session context
#[derive(Debug)]
pub enum Notification<P, V, T = SessionTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// New session notification
    NewSession(SessionContext<P, V, T>),
    /// Normal message notification
    NewMessage(Message),
}
