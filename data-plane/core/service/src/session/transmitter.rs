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
use crate::session::{
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
pub(crate) struct AppTransmitter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// SLIM tx
    pub(crate) slim_tx: SlimChannelSender,

    /// App tx
    pub(crate) app_tx: Sender<Result<Notification<P, V>, SessionError>>,

    // Interceptors to be called on message reception/send
    pub(crate) interceptors: Arc<RwLock<Vec<Arc<dyn SessionInterceptor + Send + Sync>>>>,
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

            tx.send(message.map(|msg| Notification::NewMessage(msg)))
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
