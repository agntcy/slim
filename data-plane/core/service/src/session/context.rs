// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::sync::Arc;

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};

use crate::session::common::AppChannelReceiver;
use crate::session::transmitter::SessionTransmitter;
use crate::session::{Session, Transmitter};

/// Session ID
pub type Id = u32;

/// Session context
#[derive(Debug)]
pub struct SessionContext<P, V, T = SessionTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Pointer to the session
    pub(crate) session: Arc<Session<P, V, T>>,

    /// Receive queue for the session
    pub rx: AppChannelReceiver,

    /// Optional metadata map received upon session creation
    pub metadata: Option<HashMap<String, String>>,
}

impl<P, V, T> SessionContext<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Create a new SessionContext
    pub(crate) fn new(session: Arc<Session<P, V, T>>, rx: AppChannelReceiver) -> Self {
        SessionContext {
            session,
            rx,
            metadata: None,
        }
    }

    pub(crate) fn with_metadata(self, metadata: HashMap<String, String>) -> Self {
        SessionContext {
            metadata: Some(metadata),
            ..self
        }
    }
}
