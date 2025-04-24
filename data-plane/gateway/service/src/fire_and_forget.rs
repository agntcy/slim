// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use rand::Rng;

use crate::errors::SessionError;
use crate::session::{
    AppChannelSender, Common, CommonSession, GwChannelSender, Id, MessageDirection, Session,
    SessionConfig, SessionConfigTrait, SessionDirection, SessionMessage, State,
};
use agp_datapath::messages::encoder::Agent;
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;

/// Configuration for the Fire and Forget session
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FireAndForgetConfiguration {}

impl SessionConfigTrait for FireAndForgetConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::FireAndForget(config) => {
                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected FireAndForget, got {:?}",
                session_config
            ))),
        }
    }
}

impl std::fmt::Display for FireAndForgetConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FireAndForgetConfiguration")
    }
}

/// Fire and Forget session
pub(crate) struct FireAndForget {
    common: Common,
}

impl FireAndForget {
    pub(crate) fn new(
        id: Id,
        session_config: FireAndForgetConfiguration,
        session_direction: SessionDirection,
        agent: Agent,
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> FireAndForget {
        FireAndForget {
            common: Common::new(
                id,
                session_direction,
                SessionConfig::FireAndForget(session_config),
                agent,
                tx_gw,
                tx_app,
            ),
        }
    }
}

#[async_trait]
impl Session for FireAndForget {
    async fn on_message(
        &self,
        mut message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        let header = message.message.get_session_header_mut();

        // clone tx
        match direction {
            MessageDirection::North => {
                // make sure we got a valid fnf message
                let expected = i32::from(SessionHeaderType::Fnf);
                if header.header_type != expected {
                    return Err(SessionError::ValidationError(format!(
                        "invalid header type: expected {}, got {}",
                        expected, header.header_type
                    )));
                }

                // Let's send the message to the app
                self.common
                    .tx_app_ref()
                    .send(Ok(message))
                    .await
                    .map_err(|e| SessionError::AppTransmission(e.to_string()))
            }
            MessageDirection::South => {
                // set the session type
                header.header_type = i32::from(SessionHeaderType::Fnf);
                // add a nonce to the message
                header.message_id = rand::rng().random();

                self.common
                    .tx_gw_ref()
                    .send(Ok(message.message))
                    .await
                    .map_err(|e| SessionError::GatewayTransmission(e.to_string()))
            }
        }
    }
}

delegate_common_behavior!(FireAndForget, common);

#[cfg(test)]
mod tests {
    use super::*;
    use agp_datapath::{
        messages::{Agent, AgentType},
        pubsub::ProtoMessage,
    };

    #[tokio::test]
    async fn test_fire_and_forget_create() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {},
            SessionDirection::Bidirectional,
            source,
            tx_gw,
            tx_app,
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::FireAndForget(FireAndForgetConfiguration {})
        );
    }

    #[tokio::test]
    async fn test_fire_and_forget_on_message() {
        let (tx_gw, _rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {},
            SessionDirection::Bidirectional,
            source,
            tx_gw,
            tx_app,
        );

        let mut message = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.header_type = i32::from(SessionHeaderType::Fnf);

        let res = session
            .on_message(
                SessionMessage::from(message.clone()),
                MessageDirection::North,
            )
            .await;
        assert!(res.is_ok());

        let msg = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg.message, message);
        assert_eq!(msg.info.id, 1);
    }

    #[tokio::test]
    async fn test_session_delete() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        {
            let _session = FireAndForget::new(
                0,
                FireAndForgetConfiguration {},
                SessionDirection::Bidirectional,
                source,
                tx_gw,
                tx_app,
            );
        }
    }
}
