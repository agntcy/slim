// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

use crate::{SessionError, SlimChannelSender, common::AppChannelSender};

use std::sync::Arc;

pub(crate) async fn verify_identity<V>(msg: &Message, verifier: &V) -> Result<(), SessionError>
where
    V: Verifier + Send + Sync,
{
    let identity = msg.get_slim_header().get_identity();
    if verifier.try_verify(&identity).is_err() {
        verifier.verify(&identity).await?;
    }
    Ok(())
}

pub trait MessageEncryptor: Send + Sync {
    fn encrypt_message(&self, message: &mut Message) -> Result<(), SessionError>;
}

impl<P, V> MessageEncryptor for crate::single_threaded_cell::SingleThreadedCell<crate::mls_state::MlsState<P, V>>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn encrypt_message(&self, message: &mut Message) -> Result<(), SessionError> {
        self.borrow_mut().process_message(message, crate::common::MessageDirection::South)
    }
}

#[derive(Clone)]
pub struct SessionTransmitter<P>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
{
    pub(crate) slim_tx: SlimChannelSender,
    pub(crate) app_tx: AppChannelSender,
    pub(crate) identity_provider: P,
    pub(crate) encryptor: Arc<crate::single_threaded_cell::SingleThreadedCell<Option<Arc<dyn MessageEncryptor>>>>,
}

impl<P> SessionTransmitter<P>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
{
    pub fn new(slim_tx: SlimChannelSender, app_tx: AppChannelSender, identity_provider: P) -> Self {
        SessionTransmitter {
            slim_tx,
            app_tx,
            identity_provider,
            encryptor: Arc::new(crate::single_threaded_cell::SingleThreadedCell::new(None)),
        }
    }

    pub fn set_encryptor(&self, encryptor: Arc<dyn MessageEncryptor>) {
        *self.encryptor.borrow_mut() = Some(encryptor);
    }

    pub async fn send_to_app(
        &self,
        message: Result<Message, SessionError>,
    ) -> Result<(), SessionError> {
        self.app_tx
            .send(message)
            .map_err(|_e| SessionError::ApplicationMessageSendFailed)
    }

    pub async fn send_to_slim(
        &self,
        mut message: Result<Message, Status>,
    ) -> Result<(), SessionError> {
        if let Ok(msg) = message.as_mut() {
            if msg.get_slim_header().get_identity().is_empty() {
                let identity = self.identity_provider.get_token()?;
                msg.get_slim_header_mut().set_identity(identity);
            }

            if let Some(encryptor) = &*self.encryptor.borrow() {
                encryptor.encrypt_message(msg)?;
            }
        }

        self.slim_tx
            .send(message)
            .await
            .map_err(|_e| SessionError::SlimMessageSendFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionError;
    use crate::test_utils::{MockTokenProvider, MockVerifier};
    use slim_datapath::Status;
    use slim_datapath::api::ProtoMessage as Message;
    use slim_datapath::api::ProtoName as Name;
    use tokio::sync::mpsc;

    fn make_message() -> Message {
        let source = Name::from_strings(["a", "b", "c"]).with_id(0);
        let dst = Name::from_strings(["d", "e", "f"]).with_id(0);

        Message::builder()
            .source(source)
            .destination(dst)
            .application_payload("application/octet-stream", vec![])
            .build_publish()
            .unwrap()
    }

    #[tokio::test]
    async fn session_transmitter_sets_identity_on_outbound() {
        let (slim_tx, mut slim_rx) = mpsc::channel::<Result<Message, Status>>(4);
        let (app_tx, _app_rx) = mpsc::unbounded_channel::<Result<Message, SessionError>>();
        let tx = SessionTransmitter::new(slim_tx, app_tx, MockTokenProvider);

        tx.send_to_slim(Ok(make_message())).await.unwrap();
        let sent = slim_rx.recv().await.unwrap().unwrap();
        assert_eq!(sent.get_slim_header().get_identity(), "");
    }

    #[tokio::test]
    async fn verify_identity_accepts_mock() {
        let msg = make_message();
        verify_identity(&msg, &MockVerifier).await.unwrap();
    }
}
