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

/// Transmitter used to send messages between session and application/network
#[derive(Clone)]
pub struct SessionTransmitter {
    /// SLIM tx (bounded channel)
    pub(crate) slim_tx: SlimChannelSender,

    /// App tx (unbounded channel)
    pub(crate) app_tx: AppChannelSender,
}

impl SessionTransmitter {
    pub(crate) fn new(slim_tx: SlimChannelSender, app_tx: AppChannelSender) -> Self {
        SessionTransmitter {
            slim_tx,
            app_tx,
        }
    }
}

#[async_trait::async_trait]
impl Transmitter for SessionTransmitter {
    async fn send_to_app(
        &self,
        message: Result<Message, SessionError>,
    ) -> Result<(), SessionError> {
        let tx = self.app_tx.clone();

        let ret = tx
            .send(message)
            .map_err(|_e| SessionError::ApplicationMessageSendFailed)?;

        Ok(ret)
    }

    async fn send_to_slim(&self, message: Result<Message, Status>) -> Result<(), SessionError> {
        let tx = self.slim_tx.clone();

        tx.try_send(message)
            .map_err(|_e| SessionError::SlimMessageSendFailed)
    }
}

/// Transmitter used to send messages from the application side
#[derive(Clone)]
pub struct AppTransmitter {
    /// SLIM tx (bounded channel)
    pub slim_tx: SlimChannelSender,

    /// App tx (bounded channel here; notifications)
    pub app_tx: Sender<Result<Notification, SessionError>>,

    /// Interceptors to be called on message reception/send
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

        // Apply interceptors only on successful messages
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
            .map_err(|_e| SessionError::ApplicationMessageSendFailed)
    }

    async fn send_to_slim(&self, mut message: Result<Message, Status>) -> Result<(), SessionError> {
        let tx = self.slim_tx.clone();

        // Apply interceptors only on successful messages
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
            .map_err(|_e| SessionError::SlimMessageSendFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionError, notification::Notification};
    use slim_datapath::Status;
    use slim_datapath::api::ProtoMessage as Message;
    use slim_datapath::messages::encoder::Name;
    use tokio::sync::mpsc;

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
    async fn session_transmitter_send_to_slim() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, mut app_rx) = mpsc::unbounded_channel::<Result<Message, SessionError>>();
        let tx = SessionTransmitter::new(slim_tx, app_tx);

        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let sent = slim_rx.recv().await.unwrap().unwrap();
        assert!(sent.get_payload().is_some());

        tx.send_to_app(Ok(make_message())).await.unwrap();
        let app_msg = app_rx.recv().await.unwrap().unwrap();
        assert!(app_msg.get_payload().is_some());
    }

    #[tokio::test]
    async fn session_transmitter_error_passes_through() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(1);
        let (app_tx, _app_rx) = mpsc::unbounded_channel::<Result<Message, SessionError>>();
        let tx = SessionTransmitter::new(slim_tx, app_tx);

        tx.send_to_slim(Err(Status::failed_precondition("err")))
            .await
            .unwrap();
        let result = slim_rx.recv().await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn app_transmitter_send_to_app() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, mut app_rx) = mpsc::channel::<Result<Notification, SessionError>>(4);
        let tx = AppTransmitter {
            slim_tx,
            app_tx,
            interceptors: Arc::new(RwLock::new(vec![])),
        };

        tx.send_to_app(Ok(make_message())).await.unwrap();
        if let Ok(Notification::NewMessage(msg)) = app_rx.recv().await.unwrap() {
            assert!(msg.get_payload().is_some());
        } else {
            panic!("expected NewMessage notification");
        }

        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let slim_msg = slim_rx.recv().await.unwrap().unwrap();
        assert!(slim_msg.get_payload().is_some());
    }
}