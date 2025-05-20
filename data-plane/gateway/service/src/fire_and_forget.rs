// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use agp_datapath::pubsub::{AgpHeader, SessionHeader};
use async_trait::async_trait;
use parking_lot::RwLock;
use rand::Rng;
use tracing::debug;

use crate::errors::SessionError;
use crate::session::{
    AppChannelSender, Common, CommonSession, GwChannelSender, Id, MessageDirection, Session,
    SessionConfig, SessionConfigTrait, SessionDirection, SessionMessage, State,
};
use crate::timer;
use agp_datapath::messages::encoder::Agent;
use agp_datapath::messages::utils::AgpHeaderFlags;
use agp_datapath::pubsub::proto::pubsub::v1::{Message, SessionHeaderType};

/// Configuration for the Fire and Forget session
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FireAndForgetConfiguration {
    pub timeout: Option<std::time::Duration>,
    pub max_retries: Option<u32>,
}

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
        write!(
            f,
            "FireAndForgetConfiguration: timeout: {} ms, max retries: {}",
            self.timeout.unwrap_or_default().as_millis(),
            self.max_retries.unwrap_or_default(),
        )
    }
}

/// Fire and Forget session
pub(crate) struct FireAndForgetInternal {
    common: Common,
    timers: RwLock<HashMap<u32, (timer::Timer, SessionMessage)>>,
}

#[async_trait]
impl timer::TimerObserver for FireAndForgetInternal {
    async fn on_timeout(&self, message_id: u32, _timeouts: u32) {
        // try to send the message again
        let msg = {
            let lock = self.timers.read();
            let (_timer, message) = lock.get(&message_id).expect("timer not found");
            message.message.clone()
        };

        let _ = self
            .common
            .tx_gw_ref()
            .send(Ok(msg))
            .await
            .map_err(|e| SessionError::AppTransmission(e.to_string()));
    }

    async fn on_failure(&self, message_id: u32, _timeouts: u32) {
        // remove the state for the lost message
        let (_timer, message) = self
            .timers
            .write()
            .remove(&message_id)
            .expect("timer not found");

        let _ = self
            .common
            .tx_app_ref()
            .send(Err(SessionError::Timeout {
                session_id: self.common.id(),
                message_id,
                message: Box::new(message),
            }))
            .await
            .map_err(|e| SessionError::AppTransmission(e.to_string()));
    }

    async fn on_stop(&self, message_id: u32) {
        debug!("timer stopped: {}", message_id);
    }
}

pub(crate) struct FireAndForget {
    internal: Arc<FireAndForgetInternal>,
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
        let internal = FireAndForgetInternal {
            common: Common::new(
                id,
                session_direction,
                SessionConfig::FireAndForget(session_config),
                agent,
                tx_gw,
                tx_app,
            ),
            timers: RwLock::new(HashMap::new()),
        };

        FireAndForget {
            internal: Arc::new(internal),
        }
    }

    pub(crate) async fn send_message_to_gw(
        &self,
        mut message: SessionMessage,
    ) -> Result<(), SessionError> {
        let message_id = rand::rng().random();
        let header = message.message.get_session_header_mut();
        header.set_message_id(message_id);
        message.info.set_message_id(message_id);

        // get session config
        let session_config = match self.session_config() {
            SessionConfig::FireAndForget(config) => config,
            _ => {
                return Err(SessionError::AppTransmission(
                    "invalid session config".to_string(),
                ));
            }
        };

        // create timer if needed
        if session_config.timeout.is_some() {
            header.set_header_type(SessionHeaderType::FnfReliable);
            let duration = session_config.timeout.unwrap();

            let timer = timer::Timer::new(
                message_id,
                timer::TimerType::Constant,
                duration,
                None,
                session_config.max_retries,
            );

            // start timer
            timer.start(self.internal.clone());

            // store timer and message
            self.internal
                .timers
                .write()
                .insert(message_id, (timer, message.clone()));
        } else {
            header.set_header_type(SessionHeaderType::Fnf);
        }

        // send message
        self.internal
            .common
            .tx_gw_ref()
            .send(Ok(message.message))
            .await
            .map_err(|e| SessionError::GatewayTransmission(e.to_string()))?;

        // we are good
        Ok(())
    }

    pub(crate) async fn send_message_to_app(
        &self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        let message_id = message.info.message_id.expect("message id not found");

        match message.message.get_header_type() {
            SessionHeaderType::Fnf => {
                // simply send the message tot the application
                self.internal
                    .common
                    .tx_app_ref()
                    .send(Ok(message))
                    .await
                    .map_err(|e| SessionError::GatewayTransmission(e.to_string()))?;
            }
            SessionHeaderType::FnfReliable => {
                // send an ack back as reply and forward the incoming message to the app
                // create ack message
                let src = message.message.get_source();
                let agp_header = Some(AgpHeader::new(
                    self.internal.common.source(),
                    src.agent_type(),
                    Some(src.agent_id()),
                    Some(
                        AgpHeaderFlags::default()
                            .with_forward_to(message.message.get_incoming_conn()),
                    ),
                ));

                let session_header = Some(SessionHeader::new(
                    SessionHeaderType::FnfAck.into(),
                    message.info.id,
                    message_id,
                ));

                let ack = Message::new_publish_with_headers(agp_header, session_header, "", vec![]);

                // send the ack
                self.internal
                    .common
                    .tx_gw_ref()
                    .send(Ok(ack))
                    .await
                    .map_err(|e| SessionError::GatewayTransmission(e.to_string()))?;

                // forward the message to the app
                self.internal
                    .common
                    .tx_app_ref()
                    .send(Ok(message))
                    .await
                    .map_err(|e| SessionError::GatewayTransmission(e.to_string()))?;
            }
            SessionHeaderType::FnfAck => {
                // remove the timer and drop the message
                match self.internal.timers.write().remove(&message_id) {
                    Some((mut timer, _message)) => {
                        // stop the timer
                        timer.stop();
                    }
                    None => {
                        return Err(SessionError::AppTransmission(format!(
                            "timer not found for message id {}",
                            message_id
                        )));
                    }
                }
            }
            _ => {
                // unexpected header
                return Err(SessionError::AppTransmission(
                    "invalid session header".to_string(),
                ));
            }
        }

        // we are good
        Ok(())
    }
}

#[async_trait]
impl Session for FireAndForget {
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // clone tx
        match direction {
            MessageDirection::North => self.send_message_to_app(message).await,
            MessageDirection::South => self.send_message_to_gw(message).await,
        }
    }
}

delegate_common_behavior!(FireAndForget, internal, common);

#[cfg(test)]
mod tests {
    use std::time::Duration;

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
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            source,
            tx_gw,
            tx_app,
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default())
        );
    }

    #[tokio::test]
    async fn test_fire_and_forget_on_message() {
        let (tx_gw, _rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
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
    async fn test_fire_and_forget_on_message_with_ack() {
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            source,
            tx_gw,
            tx_app,
        );

        let mut message = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            Some(AgpHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.message_id = 12345;
        header.header_type = i32::from(SessionHeaderType::FnfReliable);

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
        assert_eq!(msg.info.id, 0);
        print!("{:?}", message);

        let msg = rx_gw
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = msg.get_session_header();
        assert_eq!(header.header_type, SessionHeaderType::FnfAck.into());
        assert_eq!(header.get_message_id(), 12345);
    }

    #[tokio::test]
    async fn test_fire_and_forget_timers_until_error() {
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
            },
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

        let res = session
            .on_message(
                SessionMessage::from(message.clone()),
                MessageDirection::South,
            )
            .await;
        assert!(res.is_ok());

        // set the session id in the message for the comparison inside the for loop
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.header_type = i32::from(SessionHeaderType::FnfReliable);

        for _i in 0..6 {
            let mut msg = rx_gw
                .recv()
                .await
                .expect("no message received")
                .expect("error");
            // msg must be the same as message, except for the rundom message_id
            let header = msg.get_session_header_mut();
            header.message_id = 0;
            assert_eq!(msg, message);
        }

        let msg = rx_app.recv().await.expect("no message received");
        assert!(msg.is_err());
    }

    #[tokio::test]
    async fn test_fire_and_forget_timers_and_ack() {
        let (tx_gw_sender, mut rx_gw_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_gw_receiver, mut rx_gw_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let session_sender = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
            },
            SessionDirection::Bidirectional,
            Agent::from_strings("cisco", "default", "local_agent", 0),
            tx_gw_sender,
            tx_app_sender,
        );

        // this can be a standard fnf session
        let session_recv = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            Agent::from_strings("cisco", "default", "remote_agent", 0),
            tx_gw_receiver,
            tx_app_receiver,
        );

        let mut message = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            Some(AgpHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.set_session_id(0);
        header.set_header_type(SessionHeaderType::FnfReliable);

        let res = session_sender
            .on_message(
                SessionMessage::from(message.clone()),
                MessageDirection::South,
            )
            .await;
        assert!(res.is_ok());

        // get one message and drop it to kick in the timers
        let mut msg = rx_gw_sender
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        // msg must be the same as message, except for the rundom message_id
        let header = msg.get_session_header_mut();
        header.set_message_id(0);
        assert_eq!(msg, message);

        // this is the first RTX
        let msg = rx_gw_sender
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // this second message is received by the receiver
        let res = session_recv
            .on_message(SessionMessage::from(msg.clone()), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the message should be delivered to the app
        let mut msg = rx_app_receiver
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        // msg must be the same as message, except for the random message_id
        let header = msg.message.get_session_header_mut();
        header.set_message_id(0);
        assert_eq!(msg.message, message);

        // the session layer should generate an ack
        let ack = rx_gw_receiver
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        let header = ack.get_session_header();
        assert_eq!(header.header_type, SessionHeaderType::FnfAck.into());

        // Check that the ack is sent back to the sender
        assert_eq!(message.get_source(), ack.get_name_as_agent());

        // deliver the ack to the sender
        let res = session_sender
            .on_message(SessionMessage::from(ack.clone()), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // make sure the timer is not running anymore
        let timers = session_sender.internal.timers.read();

        // check whether the timers table contains the message id
        assert!(!timers.contains_key(&header.get_message_id()));
    }

    #[tokio::test]
    async fn test_session_delete() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        {
            let _session = FireAndForget::new(
                0,
                FireAndForgetConfiguration::default(),
                SessionDirection::Bidirectional,
                source,
                tx_gw,
                tx_app,
            );
        }
    }
}
