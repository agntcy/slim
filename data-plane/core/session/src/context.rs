// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::future::Future;
use std::sync::{Arc, Weak};

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};

use crate::common::AppChannelReceiver;
use crate::transmitter::SessionTransmitter;
use crate::{Session, Transmitter};

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
    pub fn into_parts(self) -> (Weak<Session<P, V, T>>, AppChannelReceiver) {
        (self.session, self.rx)
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
        F: FnOnce(AppChannelReceiver, Weak<Session<P, V, T>>) -> Fut + Send + 'static,
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
    use crate::handle::Session as PublicSession;
    use crate::handle::SessionType;
    use crate::interceptor::SessionInterceptor;
    use crate::interceptor::SessionInterceptorProvider;
    use crate::point_to_point::{PointToPoint, PointToPointConfiguration};
    use crate::traits::Transmitter;
    use async_trait::async_trait;
    use slim_auth::errors::AuthError;
    use slim_auth::traits::{TokenProvider, Verifier};
    use slim_datapath::Status;
    use slim_datapath::api::ProtoMessage as Message;
    use slim_datapath::messages::encoder::Name;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;

    // --- Test doubles -----------------------------------------------------------------------
    // Lightweight provider / verifier used to satisfy generic bounds of PointToPoint sessions.
    #[derive(Clone, Default)]
    struct DummyProvider;

    #[async_trait]
    impl TokenProvider for DummyProvider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }
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
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }
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

    // Minimal transmitter capturing messages; we don't assert on content here, only need a
    // concrete Transmitter implementation for constructing a real session.
    #[derive(Clone, Default)]
    struct TestTransmitter {
        app_tx: Option<AppChannelSender>,
    }

    impl SessionInterceptorProvider for TestTransmitter {
        fn add_interceptor(&self, _i: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {}
        fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
            vec![]
        }
    }

    impl Transmitter for TestTransmitter {
        #[allow(clippy::manual_async_fn)]
        fn send_to_slim(
            &self,
            _msg: Result<Message, Status>,
        ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
            async move { Ok(()) }
        }

        fn send_to_app(
            &self,
            msg: Result<Message, crate::SessionError>,
        ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
            let tx = self.app_tx.clone();
            async move {
                if let Some(tx) = tx {
                    tx.send(msg)
                        .await
                        .map_err(|e| crate::SessionError::AppTransmission(e.to_string()))?;
                }
                Ok(())
            }
        }
    }

    impl TestTransmitter {
        fn with_app_tx(app_tx: AppChannelSender) -> Self {
            Self {
                app_tx: Some(app_tx),
            }
        }
    }

    fn make_name(parts: [&str; 3]) -> Name {
        Name::from_strings(parts).with_id(0)
    }

    fn build_session_with_app_tx(
        id: u32,
        app_tx: AppChannelSender,
    ) -> Arc<PublicSession<DummyProvider, DummyVerifier, TestTransmitter>> {
        let source = make_name(["a", "b", "c"]);
        let cfg = PointToPointConfiguration::default();
        let p2p = PointToPoint::new(
            id,
            cfg.clone(),
            source,
            TestTransmitter::with_app_tx(app_tx),
            DummyProvider,
            DummyVerifier,
            std::env::temp_dir(),
        );
        Arc::new(PublicSession::from_point_to_point(p2p))
    }

    #[tokio::test]
    // Verifies that a newly created context can upgrade its Weak reference to a strong Arc
    // and exposes the expected session identity (id + type).
    async fn context_new_and_upgrade() {
        let (tx_app, rx_app) = mpsc::channel(8);
        let session = build_session_with_app_tx(1, tx_app);
        let ctx = SessionContext::new(session.clone(), rx_app);
        assert!(ctx.session_arc().is_some());
        assert_eq!(ctx.session_arc().unwrap().id(), session.id());
        assert_eq!(ctx.session_arc().unwrap().kind(), SessionType::PointToPoint);
    }

    #[tokio::test]
    // Validates spawn_receiver executes the provided closure on a background task and that
    // the Weak<Session> captured inside can still be upgraded while the original Arc exists.
    async fn context_spawn_receiver_runs_closure() {
        let (tx_app, rx_app) = mpsc::channel(4);
        let session = build_session_with_app_tx(3, tx_app);
        let ctx = SessionContext::new(session.clone(), rx_app);
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
        let session = build_session_with_app_tx(4, tx_app);
        let ctx = SessionContext::new(session.clone(), rx_app);
        let weak = ctx.spawn_receiver(|_rx, s| async move {
            let _ = s;
        });
        // Drop strong Arc
        drop(session);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(
            weak.upgrade().is_none(),
            "session should be dropped when last strong ref gone"
        );
    }

    #[tokio::test]
    // Ensures the spawned receiver task (which only reads from rx) terminates once
    // the session (and implicitly, in real usage, the channel sender owned by it) is dropped.
    async fn context_spawn_receiver_task_finishes_on_session_drop() {
        let (tx_app, rx_app) = mpsc::channel(4);
        let session = build_session_with_app_tx(5, tx_app);
        let ctx = SessionContext::new(session.clone(), rx_app);
        let (done_tx, done_rx) = oneshot::channel();
        let weak = ctx.spawn_receiver(move |mut rx, _s| async move {
            // Simply drain the channel; exit when sender side is closed.
            while rx.recv().await.is_some() {}
            let _ = done_tx.send(());
        });
        // Dropping session drops the transmitter which owns the only sender.
        drop(session);
        tokio::time::timeout(std::time::Duration::from_millis(100), done_rx)
            .await
            .expect("receiver task did not finish after session drop")
            .ok();
        assert!(weak.upgrade().is_none(), "session Arc should be gone");
    }
}
