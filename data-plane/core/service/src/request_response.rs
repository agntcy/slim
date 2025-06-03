// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use tracing::debug;

use crate::errors::SessionError;
use crate::session::{AppChannelSender, SessionConfig, SlimChannelSender};
use crate::session::{
    Common, CommonSession, Id, MessageDirection, Session, SessionConfigTrait, SessionDirection,
    State,
};
use crate::{SessionMessage, timer};
use slim_datapath::api::proto::pubsub::v1::SessionHeaderType;
use slim_datapath::messages::encoder::Agent;

/// Configuration for the Request Response session
/// This configuration is used to set the maximum number of retries and the timeout
#[derive(Debug, Clone, PartialEq)]
pub struct RequestResponseConfiguration {
    pub timeout: std::time::Duration,
}

impl SessionConfigTrait for RequestResponseConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::RequestResponse(config) => {
                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected RequestResponse, got {:?}",
                session_config
            ))),
        }
    }
}

impl Default for RequestResponseConfiguration {
    fn default() -> Self {
        RequestResponseConfiguration {
            timeout: std::time::Duration::from_millis(1000),
        }
    }
}

impl std::fmt::Display for RequestResponseConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RequestResponseConfiguration: timeout: {} ms",
            self.timeout.as_millis()
        )
    }
}

/// Internal state of the Request Response session
struct RequestResponseInternal {
    common: Common,
    timers: RwLock<HashMap<u32, (timer::Timer, SessionMessage)>>,
}

#[async_trait]
impl timer::TimerObserver for RequestResponseInternal {
    async fn on_timeout(&self, _message_id: u32, _timeouts: u32) {
        panic!("this should never happen");
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        debug_assert!(timeouts == 1);

        // get message
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

/// Request Response session
pub(crate) struct RequestResponse {
    internal: Arc<RequestResponseInternal>,
}

impl RequestResponse {
    pub(crate) fn new(
        id: Id,
        session_config: RequestResponseConfiguration,
        session_direction: SessionDirection,
        source: Agent,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
    ) -> RequestResponse {
        let internal = RequestResponseInternal {
            common: Common::new(
                id,
                session_direction,
                SessionConfig::RequestResponse(session_config),
                source,
                tx_slim,
                tx_app,
            ),
            timers: RwLock::new(HashMap::new()),
        };

        RequestResponse {
            internal: Arc::new(internal),
        }
    }

    pub(crate) async fn send_message_with_timer(
        &self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        // get message id
        let message_id = message.info.message_id.expect("message id not found");

        // get session config
        let session_config = match self.session_config() {
            SessionConfig::RequestResponse(config) => config,
            _ => {
                return Err(SessionError::AppTransmission(
                    "invalid session config".to_string(),
                ));
            }
        };

        // get duration from configuration
        let duration = session_config.timeout;

        // create new timer
        let timer = timer::Timer::new(
            message_id,
            timer::TimerType::Constant,
            duration,
            None,
            Some(0),
        );

        // send message
        self.internal
            .common
            .tx_slim_ref()
            .send(Ok(message.message.clone()))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))?;

        // start timer
        timer.start(self.internal.clone());

        // store timer and message
        self.internal
            .timers
            .write()
            .insert(message_id, (timer, message));

        // we are good
        Ok(())
    }
}

#[async_trait]
impl Session for RequestResponse {
    async fn on_message(
        &self,
        mut message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // session header
        let session_header = message.message.get_session_header_mut();

        // clone tx
        match direction {
            MessageDirection::North => {
                match message.info.session_header_type {
                    SessionHeaderType::Reply => {
                        // this is a reply - remove the timer
                        let message_id = session_header.message_id;
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
                    SessionHeaderType::Request => {
                        // this is a request - set the session_type of the session
                        // info to reply to allow the app to reply using this session info
                        message.info.session_header_type = SessionHeaderType::Reply;
                    }
                    _ => Err(SessionError::AppTransmission(format!(
                        "request/reply session: unsupported session type: {:?}",
                        message.info.session_header_type
                    )))?,
                }

                self.internal
                    .common
                    .tx_app_ref()
                    .send(Ok(message))
                    .await
                    .map_err(|e| SessionError::AppTransmission(e.to_string()))
            }
            MessageDirection::South => {
                // we are sending the message over slim.
                // Let's start setting the session header
                session_header.session_id = self.internal.common.id();
                message.info.id = self.internal.common.id();

                match message.info.session_header_type {
                    SessionHeaderType::Reply => {
                        // this is a reply - make sure the message_id matches the request
                        match message.info.message_id {
                            Some(message_id) => {
                                session_header.message_id = message_id;
                                session_header.header_type = i32::from(SessionHeaderType::Reply);

                                self.internal
                                    .common
                                    .tx_slim_ref()
                                    .send(Ok(message.message.clone()))
                                    .await
                                    .map_err(|e| SessionError::SlimTransmission(e.to_string()))
                            }
                            None => {
                                return Err(SessionError::SlimTransmission(
                                    "missing message id for reply".to_string(),
                                ));
                            }
                        }
                    }
                    _ => {
                        // In any other case, we are sending a request
                        // set the message id to something random
                        session_header.set_message_id(rand::random::<u32>());
                        session_header.set_header_type(SessionHeaderType::Request);

                        message.info.set_message_id(session_header.message_id);

                        self.send_message_with_timer(message).await
                    }
                }
            }
        }
    }
}

delegate_common_behavior!(RequestResponse, internal, common);

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::{
        api::{ProtoMessage, ProtoPublish},
        messages::{Agent, AgentType},
    };

    #[tokio::test]
    async fn test_rr_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            timeout: std::time::Duration::from_millis(1000),
        };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = RequestResponse::new(
            0,
            session_config.clone(),
            SessionDirection::Bidirectional,
            source,
            tx_slim,
            tx_app,
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::RequestResponse(session_config)
        );
    }

    #[tokio::test]
    async fn test_request_response_on_message() {
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            timeout: std::time::Duration::from_millis(1000),
        };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = RequestResponse::new(
            0,
            session_config,
            SessionDirection::Bidirectional,
            source,
            tx_slim,
            tx_app,
        );

        let payload = vec![0x1, 0x2, 0x3, 0x4];

        let mut msg = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session type to request
        let header = msg.get_session_header_mut();
        header.header_type = i32::from(SessionHeaderType::Request);

        // set the session id in the message
        header.session_id = 1;

        let session_message = SessionMessage::from(msg);

        // Send a message to the underlying slim instance
        let res = session
            .on_message(session_message, MessageDirection::South)
            .await;

        assert!(res.is_ok(), "{}", res.unwrap_err());

        // we will wait for a response, but as no one is reply, we will get a timeout
        let res = rx_app.recv().await.expect("no message received");
        assert!(res.is_err(), "{}", res.unwrap_err());

        // We also should get the message back in the error
        let err = res.unwrap_err();
        match err {
            SessionError::Timeout {
                session_id,
                message,
                ..
            } => {
                let blob = ProtoPublish::from(message.message)
                    .msg
                    .as_ref()
                    .expect("error getting message")
                    .blob
                    .clone();
                assert_eq!(blob, payload);
                assert_eq!(session_id, session.id());
            }
            _ => panic!("unexpected error"),
        }
    }

    #[tokio::test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            timeout: std::time::Duration::from_millis(1000),
        };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        {
            let _session = RequestResponse::new(
                0,
                session_config,
                SessionDirection::Bidirectional,
                source,
                tx_slim,
                tx_app,
            );
        }
    }
}
