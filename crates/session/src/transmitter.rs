// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

use crate::{SessionError, SlimChannelSender, common::AppChannelSender};

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

#[derive(Clone)]
pub struct SessionTransmitter<P>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
{
    pub(crate) slim_tx: SlimChannelSender,
    pub(crate) app_tx: AppChannelSender,
    pub(crate) identity_provider: P,
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
        }
    }
}

impl<P> SessionTransmitter<P>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
{
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
            let identity = self.identity_provider.get_token()?;
            msg.get_slim_header_mut().set_identity(identity);
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
