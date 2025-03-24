// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::session::SessionType;
use crate::session::{Common, Error, Id, Info, MessageDirection, Session, SessionDirection, State};

use rand::Rng;
use tonic::Status;

use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;

pub(crate) struct FireAndForget {
    common: Common,
}

impl FireAndForget {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
    ) -> FireAndForget {
        FireAndForget {
            common: Common::new(id, session_direction, tx_gw, tx_app),
        }
    }
}

impl Session for FireAndForget {
    fn id(&self) -> Id {
        self.common.id()
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_type(&self) -> SessionType {
        SessionType::FireAndForget
    }

    fn on_message(
        &self,
        mut message: Message,
        direction: MessageDirection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        // set the session type
        let header = utils::get_session_header_as_mut(&mut message);
        if header.is_none() {
            return Box::pin(
                async move { Err(Error::AppTransmission("missing header".to_string())) },
            );
        }

        header.unwrap().header_type = utils::service_type_to_int(SessionHeaderType::CtrlFnf);

        // clone tx
        match direction {
            MessageDirection::North => {
                // create info
                let info = Info::new(
                    self.common.id(),
                    SessionType::FireAndForget,
                    self.common.state().clone(),
                );

                let tx = self.common.tx_app();
                Box::pin(async move {
                    tx.send((message, info))
                        .await
                        .map_err(|e| Error::AppTransmission(e.to_string()))
                })
            }
            MessageDirection::South => {
                // add a nonce to the message
                let header = utils::get_session_header_as_mut(&mut message).unwrap();
                header.message_id = rand::rng().random();

                let tx = self.common.tx_gw();
                Box::pin(async move {
                    tx.send(Ok(message))
                        .await
                        .map_err(|e| Error::GatewayTransmission(e.to_string()))
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agp_datapath::messages::encoder;

    #[tokio::test]
    async fn test_fire_and_forget_create() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session = FireAndForget::new(0, SessionDirection::Bidirectional, tx_gw, tx_app);

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(session.session_type(), SessionType::FireAndForget);
    }

    #[tokio::test]
    async fn test_fire_and_forget_on_message() {
        let (tx_gw, _rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session = FireAndForget::new(0, SessionDirection::Bidirectional, tx_gw, tx_app);

        let mut message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
        header.session_id = 1;

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let (msg, info) = rx_app.recv().await.unwrap();
        assert_eq!(msg, message);
        assert_eq!(info.id, 0);
        assert_eq!(info.session_type, SessionType::FireAndForget);
        assert_eq!(info.state, State::Active);
    }
}
