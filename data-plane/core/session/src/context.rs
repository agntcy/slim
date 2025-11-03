// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::future::Future;
use std::sync::{Arc, Weak};

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};

use crate::common::AppChannelReceiver;
use crate::session_controller::SessionController;

/// Session context
#[derive(Debug)]
pub struct SessionContext<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Weak reference to session (lifecycle managed externally)
    pub session: Weak<SessionController<P, V>>,

    /// Receive queue for the session
    pub rx: AppChannelReceiver,
}

impl<P, V> SessionContext<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new SessionContext
    pub fn new(session: Arc<SessionController<P, V>>, rx: AppChannelReceiver) -> Self {
        SessionContext {
            session: Arc::downgrade(&session),
            rx,
        }
    }

    /// Get a weak reference to the underlying session handle.
    pub fn session(&self) -> &Weak<SessionController<P, V>> {
        &self.session
    }

    /// Get a Arc to the underlying session handle
    pub fn session_arc(&self) -> Option<Arc<SessionController<P, V>>> {
        self.session().upgrade()
    }

    /// Consume the context returning session, receiver and optional metadata.
    pub fn into_parts(self) -> (Weak<SessionController<P, V>>, AppChannelReceiver) {
        (self.session, self.rx)
    }

    /// Spawn a Tokio task to process the receive channel while returning the session handle.
    ///
    /// The provided closure receives ownership of the `AppChannelReceiver`, a `Weak<SessionController>` and
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
    pub fn spawn_receiver<F, Fut>(self, f: F) -> Weak<SessionController<P, V>>
    where
        F: FnOnce(AppChannelReceiver, Weak<SessionController<P, V>>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (session, rx) = self.into_parts();
        let session_clone = session.clone();
        tokio::spawn(async move {
            f(rx, session_clone).await;
        });
        session
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionError;
    use crate::common::AppChannelSender;
    use crate::session_controller::{SessionConfig, SessionController};
    use crate::transmitter::SessionTransmitter;
    use async_trait::async_trait;
    use slim_auth::errors::AuthError;
    use slim_auth::traits::{TokenProvider, Verifier};
    use slim_datapath::api::ProtoSessionType;
    use slim_datapath::messages::Name;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;

    // --- Test doubles -----------------------------------------------------------------------
    // Lightweight provider / verifier used to satisfy generic bounds of sessions.
    #[derive(Clone, Default)]
    struct DummyProvider;
    impl TokenProvider for DummyProvider {
        fn get_token(&self) -> Result<String, AuthError> {
            Ok("t".into())
        }

        fn get_id(&self) -> Result<String, AuthError> {
            Ok("id".into())
        }
    }
    #[derive(Clone, Default)]
    struct DummyVerifier;
    #[async_trait]
    impl Verifier for DummyVerifier {
        async fn verify(&self, _t: impl Into<String> + Send) -> Result<(), AuthError> {
            Ok(())
        }
        fn try_verify(&self, _t: impl Into<String>) -> Result<(), AuthError> {
            Ok(())
        }
        async fn get_claims<Claims>(
            &self,
            _t: impl Into<String> + Send,
        ) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("na".into()))
        }
        fn try_get_claims<Claims>(&self, _t: impl Into<String>) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("na".into()))
        }
    }

    fn make_name(parts: [&str; 3]) -> Name {
        Name::from_strings(parts).with_id(0)
    }

    async fn build_session_controller_with_app_tx(
        id: u32,
        app_tx: AppChannelSender,
    ) -> Arc<SessionController<DummyProvider, DummyVerifier>> {
        use crate::SlimChannelSender;
        use crate::common::SessionMessage;

        let source = make_name(["a", "b", "c"]);
        let destination = make_name(["x", "y", "z"]);
        let cfg = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            duration: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: false,
            metadata: Default::default(),
        };

        // Create channels for SessionTransmitter
        let (slim_tx, _slim_rx): (SlimChannelSender, _) = mpsc::channel(32);

        // Create a SessionTransmitter
        let session_tx = SessionTransmitter::new(slim_tx, app_tx.clone());

        // Create channel for session layer communication
        let (tx_session, _rx_session): (mpsc::Sender<Result<SessionMessage, SessionError>>, _) =
            mpsc::channel(32);

        // Create a SessionController
        Arc::new(
            SessionController::new(
                id,
                source,
                destination,
                cfg,
                DummyProvider,
                DummyVerifier,
                std::env::temp_dir(),
                session_tx,
                tx_session,
            )
            .await,
        )
    }

    #[tokio::test]
    // Verifies that a newly created context can upgrade its Weak reference to a strong Arc
    // and exposes the expected session identity (id + type).
    async fn context_new_and_upgrade() {
        let (tx_app, rx_app) = mpsc::channel(8);
        let session_controller = build_session_controller_with_app_tx(1, tx_app).await;
        let ctx = SessionContext::new(session_controller.clone(), rx_app);
        assert!(ctx.session_arc().is_some());
    }

    #[tokio::test]
    // Validates spawn_receiver executes the provided closure on a background task and that
    // the Weak<Session> captured inside can still be upgraded while the original Arc exists.
    async fn context_spawn_receiver_runs_closure() {
        let (tx_app, rx_app) = mpsc::channel(4);
        let session_controller = build_session_controller_with_app_tx(3, tx_app).await;
        let ctx = SessionContext::new(session_controller.clone(), rx_app);
        let flag = Arc::new(tokio::sync::Mutex::new(false));
        let flag_clone = flag.clone();
        let weak = ctx.spawn_receiver(move |_rx, s| async move {
            assert!(s.upgrade().is_some());
            *flag_clone.lock().await = true;
        });
        assert!(weak.upgrade().is_some());
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(*flag.lock().await, "closure not executed");
    }

    #[tokio::test]
    // After spawning the receiver, dropping the last strong Arc should allow the Weak to
    // observe session deallocation (upgrade returns None).
    async fn context_spawn_receiver_drops_session() {
        let (tx_app, rx_app) = mpsc::channel(4);
        let session_controller = build_session_controller_with_app_tx(4, tx_app).await;
        let ctx = SessionContext::new(session_controller.clone(), rx_app);
        let weak = ctx.spawn_receiver(|_rx, s| async move {
            let _ = s;
        });
        // Drop strong Arc
        drop(session_controller);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(
            weak.upgrade().is_none(),
            "session should be dropped when last strong ref gone"
        );
    }

    #[tokio::test]
    // Ensures the spawned receiver task (which only reads from rx) terminates once
    // the channel is explicitly closed (e.g., by dropping the sender).
    async fn context_spawn_receiver_task_finishes_on_session_drop() {
        let (tx_app, rx_app) = mpsc::channel(4);
        let session_controller = build_session_controller_with_app_tx(5, tx_app.clone()).await;
        let ctx = SessionContext::new(session_controller.clone(), rx_app);
        let (done_tx, done_rx) = oneshot::channel();
        let weak = ctx.spawn_receiver(move |mut rx, _s| async move {
            // Simply drain the channel; exit when sender side is closed.
            while rx.recv().await.is_some() {}
            let _ = done_tx.send(());
        });
        // Drop both the sender and session controller to close the channel
        drop(tx_app);
        drop(session_controller);
        tokio::time::timeout(std::time::Duration::from_millis(200), done_rx)
            .await
            .expect("receiver task did not finish after channel close")
            .ok();
        assert!(weak.upgrade().is_none(), "session Arc should be gone");
    }
}
