// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};

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
    /// Weak reference to session (lifecycle managed externally)
    pub session: Weak<Session<P, V, T>>,

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
    pub fn new(session: Arc<Session<P, V, T>>, rx: AppChannelReceiver) -> Self {
        SessionContext {
            session: Arc::downgrade(&session),
            rx,
            metadata: None,
        }
    }

    pub fn with_metadata(self, metadata: HashMap<String, String>) -> Self {
        SessionContext {
            metadata: Some(metadata),
            ..self
        }
    }

    /// Get a weak reference to the underlying session handle.
    pub fn session(&self) -> &Weak<Session<P, V, T>> {
        &self.session
    }

    /// Get a Arc to the underlying session handle
    pub fn session_arc(&self) -> Option<Arc<Session<P, V, T>>> {
        self.session().upgrade()
    }

    /// Consume the context returning session, receiver and optional metadata.
    pub fn into_parts(
        self,
    ) -> (
        Weak<Session<P, V, T>>,
        AppChannelReceiver,
        Option<HashMap<String, String>>,
    ) {
        (self.session, self.rx, self.metadata)
    }

    /// Spawn a Tokio task to process the receive channel while returning the session handle.
    ///
    /// The provided closure receives ownership of the `AppChannelReceiver`, a `Weak<Session>` and
    /// the optional metadata. It runs inside a `tokio::spawn` so any panic will be isolated.
    ///
    /// Example usage:
    /// ```ignore
    /// let session = ctx.spawn_receiver(|mut rx, session, _meta| async move {
    ///     while let Some(Ok(msg)) = rx.recv().await {
    ///         // handle msg with session
    ///     }
    /// });
    /// // keep using `session` for lifecycle operations (e.g. deletion)
    /// ```
    pub fn spawn_receiver<F, Fut>(self, f: F) -> Weak<Session<P, V, T>>
    where
        F: FnOnce(
                AppChannelReceiver,
                Weak<Session<P, V, T>>,
                Option<HashMap<String, String>>,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (session, rx, metadata) = self.into_parts();
        let session_clone = session.clone();
        tokio::spawn(async move {
            f(rx, session_clone, metadata).await;
        });
        session
    }
}
