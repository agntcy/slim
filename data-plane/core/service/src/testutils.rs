#![cfg(test)]

use std::sync::Arc;

use tokio::sync::mpsc::error::SendError;

use crate::{
    SessionMessage,
    errors::SessionError,
    interceptor::{SessionInterceptor, SessionInterceptorProvider},
    session::{AppChannelSender, SessionTransmitter, SlimChannelSender},
};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

#[derive(Clone)]
pub(crate) struct MockTransmitter {
    pub(crate) tx_app: AppChannelSender,
    pub(crate) tx_slim: SlimChannelSender,
}

impl SessionTransmitter for MockTransmitter {
    fn send_to_app(
        &self,
        message: Result<SessionMessage, SessionError>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        let tx = self.tx_app.clone();
        async move {
            tx.send(message)
                .await
                .map_err(|e: SendError<Result<SessionMessage, SessionError>>| {
                    SessionError::AppTransmission(e.to_string())
                })
        }
    }

    fn send_to_slim(
        &self,
        message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static {
        let tx = self.tx_slim.clone();
        async move {
            tx.send(message)
                .await
                .map_err(|e: SendError<Result<Message, Status>>| {
                    SessionError::SlimTransmission(e.to_string())
                })
        }
    }
}

impl SessionInterceptorProvider for MockTransmitter {
    fn add_interceptor(&self, _interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        // Mock implementation does not store interceptors
        // In a real implementation, you would store the interceptor in a collection
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        // Mock implementation returns an empty vector
        // In a real implementation, you would return the stored interceptors
        Vec::new()
    }

    fn derive_new(&self) -> Self {
        MockTransmitter {
            tx_app: self.tx_app.clone(),
            tx_slim: self.tx_slim.clone(),
        }
    }
}
