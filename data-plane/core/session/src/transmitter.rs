// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock;
use slim_auth::traits::{TokenProvider, Verifier};
use tokio::sync::mpsc::Sender;

use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use crate::{
    SessionError, SlimChannelSender, Transmitter,
    common::AppChannelSender,
    interceptor::{SessionInterceptor, SessionInterceptorProvider},
    notification::Notification,
};

/// Macro to generate the common transmitter method body pattern
macro_rules! transmit_with_interceptors {
    (
        $self:ident,
        $message:ident,
        $tx_field:ident,
        $interceptor_method:ident,
        $error_variant:ident
    ) => {{
        let tx = $self.$tx_field.clone();

        // Interceptors
        let interceptors = match &$message {
            Ok(_) => $self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        async move {
            if let Ok(msg) = $message.as_mut() {
                // Apply interceptors on the message
                for interceptor in interceptors {
                    interceptor.$interceptor_method(msg).await?;
                }
            }

            tx.send($message)
                .await
                .map_err(|e| SessionError::$error_variant(e.to_string()))
        }
    }};
}

/// Transmitter used to intercept messages sent from sessions and apply interceptors on them
#[derive(Clone)]
pub struct SessionTransmitter {
    /// SLIM tx
    pub(crate) slim_tx: SlimChannelSender,

    /// App tx
    pub(crate) app_tx: AppChannelSender,

    // Interceptors to be called on message reception/send
    pub(crate) interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync>>>>,
}

impl SessionTransmitter {
    pub(crate) fn new(slim_tx: SlimChannelSender, app_tx: AppChannelSender) -> Self {
        SessionTransmitter {
            slim_tx,
            app_tx,
            interceptors: Arc::new(RwLock::new(vec![])),
        }
    }
}

impl SessionInterceptorProvider for SessionTransmitter {
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.write().push(interceptor);
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        self.interceptors.read().clone()
    }
}

impl Transmitter for SessionTransmitter {
    fn send_to_app(
        &self,
        mut message: Result<Message, SessionError>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        transmit_with_interceptors!(self, message, app_tx, on_msg_from_slim, AppTransmission)
    }

    fn send_to_slim(
        &self,
        mut message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        transmit_with_interceptors!(self, message, slim_tx, on_msg_from_app, SlimTransmission)
    }
}

/// Transmitter used to intercept messages sent from sessions and apply interceptors on them
#[derive(Clone)]
pub struct AppTransmitter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// SLIM tx
    pub slim_tx: SlimChannelSender,

    /// App tx
    pub app_tx: Sender<Result<Notification<P, V>, SessionError>>,

    // Interceptors to be called on message reception/send
    pub interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync>>>>,
}

impl<P, V> SessionInterceptorProvider for AppTransmitter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.write().push(interceptor);
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        self.interceptors.read().clone()
    }
}

impl<P, V> Transmitter for AppTransmitter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn send_to_app(
        &self,
        mut message: Result<Message, SessionError>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        let tx = self.app_tx.clone();

        // Interceptors
        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        async move {
            if let Ok(msg) = message.as_mut() {
                // Apply interceptors on the message
                for interceptor in interceptors {
                    interceptor.on_msg_from_slim(msg).await?;
                }
            }

            tx.send(message.map(|msg| Notification::NewMessage(Box::new(msg))))
                .await
                .map_err(|e| SessionError::AppTransmission(e.to_string()))
        }
    }

    fn send_to_slim(
        &self,
        mut message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        transmit_with_interceptors!(self, message, slim_tx, on_msg_from_app, SlimTransmission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interceptor::{SessionInterceptor, SessionInterceptorProvider};
    use crate::{SessionError, notification::Notification};
    use async_trait::async_trait;
    use slim_auth::errors::AuthError;
    use slim_auth::traits::{TokenProvider, Verifier};
    use slim_datapath::Status;
    use slim_datapath::api::{ApplicationPayload, ProtoMessage as Message};
    use slim_datapath::messages::encoder::Name;
    use tokio::sync::mpsc;

    #[derive(Clone, Default)]
    struct DummyProvider;
    impl TokenProvider for DummyProvider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }

        fn get_token(&self) -> Result<String, AuthError> {
            Ok("id-token".into())
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
        
        async fn verify(&self, _token: impl Into<String> + Send) -> Result<(), AuthError> {
            Ok(())
        }
        fn try_verify(&self, _token: impl Into<String>) -> Result<(), AuthError> {
            Ok(())
        }
        async fn get_claims<Claims>(
            &self,
            _token: impl Into<String> + Send,
        ) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("na".into()))
        }
        fn try_get_claims<Claims>(&self, _token: impl Into<String>) -> Result<Claims, AuthError>
        where
            Claims: serde::de::DeserializeOwned + Send,
        {
            Err(AuthError::TokenInvalid("na".into()))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingInterceptor {
        pub app_calls: Arc<RwLock<usize>>,
        pub slim_calls: Arc<RwLock<usize>>,
    }

    #[async_trait]
    impl SessionInterceptor for RecordingInterceptor {
        async fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError> {
            *self.app_calls.write() += 1;
            msg.insert_metadata("APP".into(), "1".into());
            Ok(())
        }
        async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
            *self.slim_calls.write() += 1;
            msg.insert_metadata("SLIM".into(), "1".into());
            Ok(())
        }
    }

    fn make_message() -> Message {
        let source = Name::from_strings(["a", "b", "c"]).with_id(0);
        let dst = Name::from_strings(["d", "e", "f"]).with_id(0);
        // Signature: (&Name, &Name, Option<SlimHeaderFlags>, &str, Vec<u8>)
        let payload =
            Some(ApplicationPayload::new("application/octet-stream", vec![]).as_content());
        Message::new_publish(&source, &dst, None, None, payload)
    }

    #[tokio::test]
    async fn session_transmitter_interceptor_application_send_to_slim() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, mut app_rx) = mpsc::channel::<Result<Message, SessionError>>(4);
        let tx = SessionTransmitter::new(slim_tx, app_tx);
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

        // send_to_slim treats the message as originating from the app -> on_msg_from_app invoked
        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let sent = slim_rx.recv().await.unwrap().unwrap();
        assert_eq!(sent.get_metadata("APP").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.app_calls.read(), 1);
        assert_eq!(*interceptor.slim_calls.read(), 0);

        // send_to_app treats the message as coming from slim -> on_msg_from_slim invoked
        tx.send_to_app(Ok(make_message())).await.unwrap();
        let app_msg = app_rx.recv().await.unwrap().unwrap();
        assert_eq!(app_msg.get_metadata("SLIM").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.slim_calls.read(), 1); // first slim direction call
    }

    #[tokio::test]
    async fn session_transmitter_error_bypasses_interceptors() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(1);
        let (app_tx, _app_rx) = mpsc::channel::<Result<Message, SessionError>>(1);
        let tx = SessionTransmitter::new(slim_tx, app_tx);
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

        // Error result: interceptors should not run, calls remain 0
        tx.send_to_slim(Err(Status::failed_precondition("err")))
            .await
            .unwrap();
        let _ = slim_rx.recv().await.unwrap();
        assert_eq!(*interceptor.slim_calls.read(), 0);
        assert_eq!(*interceptor.app_calls.read(), 0);
    }

    #[tokio::test]
    async fn app_transmitter_interceptor_application_send_to_app() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, mut app_rx) =
            mpsc::channel::<Result<Notification<DummyProvider, DummyVerifier>, SessionError>>(4);
        let tx = AppTransmitter::<DummyProvider, DummyVerifier> {
            slim_tx,
            app_tx,
            interceptors: Arc::new(RwLock::new(vec![])),
        };
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

        // AppTransmitter::send_to_app uses on_msg_from_slim (message inbound from slim)
        tx.send_to_app(Ok(make_message())).await.unwrap();
        if let Ok(Notification::NewMessage(msg)) = app_rx.recv().await.unwrap() {
            assert_eq!(msg.get_metadata("SLIM").map(|s| s.as_str()), Some("1"));
            assert_eq!(*interceptor.slim_calls.read(), 1);
            assert_eq!(*interceptor.app_calls.read(), 0);
        } else {
            panic!("expected NewMessage notification");
        }

        // AppTransmitter::send_to_slim uses on_msg_from_app (message outbound to slim)
        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let slim_msg = slim_rx.recv().await.unwrap().unwrap();
        assert_eq!(slim_msg.get_metadata("APP").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.app_calls.read(), 1);
    }
}
