// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock;
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

/// Transmitter used to intercept messages sent from sessions and apply interceptors on them
#[derive(Clone)]
pub struct SessionTransmitter {
    /// SLIM tx (bounded channel)
    pub(crate) slim_tx: SlimChannelSender,

    /// App tx (unbounded channel)
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

#[async_trait::async_trait]
impl Transmitter for SessionTransmitter {
    async fn send_to_app(
        &self,
        mut message: Result<Message, SessionError>,
    ) -> Result<(), SessionError> {
        let tx = self.app_tx.clone();

        // Interceptors only run on successful messages
        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        if let Ok(msg) = message.as_mut() {
            for interceptor in interceptors {
                interceptor.on_msg_from_slim(msg).await?;
            }
        }

        tx.send(message)
            .map_err(|e| SessionError::AppTransmission(e.to_string()))
    }

    async fn send_to_slim(&self, mut message: Result<Message, Status>) -> Result<(), SessionError> {
        let tx = self.slim_tx.clone();

        // Interceptors only run on successful messages
        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        if let Ok(msg) = message.as_mut() {
            for interceptor in interceptors {
                interceptor.on_msg_from_app(msg).await?;
            }
        }

        tx.try_send(message)
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }
}

/// Transmitter used to intercept messages sent from the application side and apply interceptors
#[derive(Clone)]
pub struct AppTransmitter {
    /// SLIM tx (bounded channel)
    pub slim_tx: SlimChannelSender,

    /// App tx (bounded channel here; notifications)
    pub app_tx: Sender<Result<Notification, SessionError>>,

    // Interceptors to be called on message reception/send
    pub interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync>>>>,
}

impl SessionInterceptorProvider for AppTransmitter {
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.write().push(interceptor);
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        self.interceptors.read().clone()
    }
}

#[async_trait::async_trait]
impl Transmitter for AppTransmitter {
    async fn send_to_app(
        &self,
        mut message: Result<Message, SessionError>,
    ) -> Result<(), SessionError> {
        let tx = self.app_tx.clone();

        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        if let Ok(msg) = message.as_mut() {
            for interceptor in interceptors {
                interceptor.on_msg_from_slim(msg).await?;
            }
        }

        tx.send(message.map(|msg| Notification::NewMessage(Box::new(msg))))
            .await
            .map_err(|e| SessionError::AppTransmission(e.to_string()))
    }

    async fn send_to_slim(&self, mut message: Result<Message, Status>) -> Result<(), SessionError> {
        let tx = self.slim_tx.clone();

        // Interceptors only run on successful messages
        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        if let Ok(msg) = message.as_mut() {
            for interceptor in interceptors {
                interceptor.on_msg_from_app(msg).await?;
            }
        }

        tx.try_send(message)
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interceptor::{SessionInterceptor, SessionInterceptorProvider};
    use crate::{SessionError, notification::Notification};
    use async_trait::async_trait;
    use slim_datapath::Status;
    use slim_datapath::api::ProtoMessage as Message;
    use slim_datapath::messages::encoder::Name;
    use tokio::sync::mpsc;

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
        Message::builder()
            .source(source)
            .destination(dst)
            .application_payload("application/octet-stream", vec![])
            .build_publish()
            .unwrap()
    }

    #[tokio::test]
    async fn session_transmitter_interceptor_application_send_to_slim() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, mut app_rx) = mpsc::unbounded_channel::<Result<Message, SessionError>>();
        let tx = SessionTransmitter::new(slim_tx, app_tx);
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let sent = slim_rx.recv().await.unwrap().unwrap();
        assert_eq!(sent.get_metadata("APP").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.app_calls.read(), 1);
        assert_eq!(*interceptor.slim_calls.read(), 0);

        tx.send_to_app(Ok(make_message())).await.unwrap();
        let app_msg = app_rx.recv().await.unwrap().unwrap();
        assert_eq!(app_msg.get_metadata("SLIM").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.slim_calls.read(), 1);
    }

    #[tokio::test]
    async fn session_transmitter_error_bypasses_interceptors() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(1);
        let (app_tx, _app_rx) = mpsc::unbounded_channel::<Result<Message, SessionError>>();
        let tx = SessionTransmitter::new(slim_tx, app_tx);
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

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
        let (app_tx, mut app_rx) = mpsc::channel::<Result<Notification, SessionError>>(4);
        let tx = AppTransmitter {
            slim_tx,
            app_tx,
            interceptors: Arc::new(RwLock::new(vec![])),
        };
        let interceptor = Arc::new(RecordingInterceptor::default());
        tx.add_interceptor(interceptor.clone());

        tx.send_to_app(Ok(make_message())).await.unwrap();
        if let Ok(Notification::NewMessage(msg)) = app_rx.recv().await.unwrap() {
            assert_eq!(msg.get_metadata("SLIM").map(|s| s.as_str()), Some("1"));
            assert_eq!(*interceptor.slim_calls.read(), 1);
            assert_eq!(*interceptor.app_calls.read(), 0);
        } else {
            panic!("expected NewMessage notification");
        }

        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let slim_msg = slim_rx.recv().await.unwrap().unwrap();
        assert_eq!(slim_msg.get_metadata("APP").map(|s| s.as_str()), Some("1"));
        assert_eq!(*interceptor.app_calls.read(), 1);
    }
}
