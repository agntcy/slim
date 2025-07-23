// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::RwLock;
use slim_datapath::api::ProtoMessage as Message;
use tokio::sync::mpsc::error::SendError;

use crate::{
    SessionMessage,
    errors::SessionError,
    interceptor::{SessionInterceptor, SessionInterceptorProvider},
    session::{AppChannelSender, SessionTransmitter, SlimChannelSender},
};
use slim_datapath::Status;

/// Transmitter used to intercept messages sent from sessions and apply interceptors on them
#[derive(Clone)]
pub(crate) struct Transmitter {
    /// SLIM tx
    pub(crate) slim_tx: SlimChannelSender,

    /// Application tx
    pub(crate) app_tx: AppChannelSender,

    // Interceptors to be called on message reception/send
    pub(crate) interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync>>>>,
}

impl SessionInterceptorProvider for Transmitter {
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.write().push(interceptor);
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        self.interceptors.read().clone()
    }

    fn derive_new(&self) -> Self {
        Transmitter {
            slim_tx: self.slim_tx.clone(),
            app_tx: self.app_tx.clone(),
            interceptors: Arc::new(RwLock::new(self.interceptors.read().clone())),
        }
    }
}

impl SessionTransmitter for Transmitter {
    fn send_to_app(
        &self,
        mut message: Result<SessionMessage, SessionError>,
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
                    interceptor.on_msg_from_slim(&mut msg.message).await?;
                }
            }

            tx.send(message)
                .await
                .map_err(|e: SendError<Result<SessionMessage, SessionError>>| {
                    SessionError::AppTransmission(e.to_string())
                })
        }
    }

    fn send_to_slim(
        &self,
        mut message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        let tx = self.slim_tx.clone();

        // Interceptors
        let interceptors = match &message {
            Ok(_) => self.interceptors.read().clone(),
            Err(_) => Vec::new(),
        };

        async move {
            if let Ok(msg) = message.as_mut() {
                // Apply interceptors on the message
                for interceptor in interceptors {
                    interceptor.on_msg_from_app(msg).await?;
                }
            }

            tx.send(message)
                .await
                .map_err(|e: SendError<Result<Message, Status>>| {
                    SessionError::SlimTransmission(e.to_string())
                })
        }
    }
}
