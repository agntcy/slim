// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use tracing::{debug, error};

use crate::errors::SessionError;
use crate::session::{AppChannelSender, GwChannelSender, SessionConfig};
use crate::session::{
    Common, CommonSession, Id, MessageDirection, Session, SessionDirection, State,
};
use crate::{timer, SessionMessage};
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;

/// Configuration for the Request Response session
/// This configuration is used to set the maximum number of retries and the timeout
#[derive(Debug, Clone, PartialEq)]
pub struct RequestResponseConfiguration {
    pub max_retries: u32,
    pub timeout: std::time::Duration,
}

impl Default for RequestResponseConfiguration {
    fn default() -> Self {
        RequestResponseConfiguration {
            max_retries: 0,
            timeout: std::time::Duration::from_millis(1000),
        }
    }
}

impl std::fmt::Display for RequestResponseConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RequestResponseConfiguration: max_retries: {}, timeout: {} ms",
            self.max_retries,
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
                error: message_id.to_string(),
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
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> RequestResponse {
        let internal = RequestResponseInternal {
            common: Common::new(
                id,
                session_direction,
                SessionConfig::RequestResponse(session_config),
                tx_gw,
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

        // create new timer
        let timer = timer::Timer::new(message_id, 1000, 0);

        // send message
        self.internal
            .common
            .tx_gw_ref()
            .send(Ok(message.message.clone()))
            .await
            .map_err(|e| SessionError::GatewayTransmission(e.to_string()))?;

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
        let session_header = message.message.session_header_mut();

        // clone tx
        match direction {
            MessageDirection::North => {
                match message.info.session_header_type {
                    SessionHeaderType::Reply => {
                        // this is a reply - remove the timer
                        let message_id = session_header.message_id;
                        match self.internal.timers.write().remove(&message_id) {
                            Some((timer, _message)) => {
                                // stop the timer
                                timer.stop();
                            }
                            None => {
                                return Err(SessionError::AppTransmission(format!(
                                    "timer not found for message id {}",
                                    message_id
                                )))
                            }
                        }
                    }
                    SessionHeaderType::Request => {
                        // this is a request - set the session_type pf the session
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
                // we are sending the message over the gateway.
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
                                    .tx_gw_ref()
                                    .send(Ok(message.message.clone()))
                                    .await
                                    .map_err(|e| SessionError::GatewayTransmission(e.to_string()))
                            }
                            None => {
                                return Err(SessionError::GatewayTransmission(
                                    "missing message id for reply".to_string(),
                                ))
                            }
                        }
                    }
                    _ => {
                        // In any other case, we are sending a request
                        // set the message id to something random
                        session_header.message_id = rand::random::<u32>();
                        message.info.message_id = Some(session_header.message_id);
                        session_header.header_type = i32::from(SessionHeaderType::Request);

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
    use agp_datapath::{
        messages::{Agent, AgentType},
        pubsub::{ProtoMessage, ProtoPublish},
    };

    #[tokio::test]
    async fn test_rr_create() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            max_retries: 0,
            timeout: std::time::Duration::from_millis(1000),
        };

        let session = RequestResponse::new(
            0,
            session_config.clone(),
            SessionDirection::Bidirectional,
            tx_gw,
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
    async fn test_request_response() {
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            max_retries: 0,
            timeout: std::time::Duration::from_millis(100),
        };

        // create a new session
        let session = RequestResponse::new(
            0,
            session_config,
            SessionDirection::Bidirectional,
            tx_gw,
            tx_app,
        );

        let request_msg = "thisistherequest";
        let response_msg = "thisistheresponse";

        let msg = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            request_msg.as_bytes().to_vec(),
        );

        let mut session_message = SessionMessage::from(msg);
        session_message.info.id = session.id();

        // Send a message to the underlying gateway
        let res = session
            .on_message(session_message, MessageDirection::South)
            .await;

        assert!(res.is_ok(), "{}", res.unwrap_err());

        // Let's receive the message and send a response back to the app
        let msg = rx_gw
            .recv()
            .await
            .expect("no message received")
            .expect("error receiving message");

        // Make sure this is a request
        let header = msg.session_header();
        assert_eq!(header.header_type, i32::from(SessionHeaderType::Request));
        assert_eq!(header.session_id, session.id());

        // Create the session message starting from the received message
        let mut session_message = SessionMessage::from(msg);

        // Send a reply back to the app
        let mut reply = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            response_msg.as_bytes().to_vec(),
        );

        // Get the reply header
        let header = reply.session_header_mut();

        // Manually set message_id and header_type as this message is not passing
        // through the gateway
        header.message_id = session_message.info.message_id.unwrap();
        header.header_type = i32::from(SessionHeaderType::Reply);

        // use the same session message to send the reply
        session_message.message = reply;

        // Send the message to the app
        let res = session
            .on_message(session_message, MessageDirection::North)
            .await;
        assert!(res.is_ok(), "{}", res.unwrap_err());

        // The message should be received by the app
        let res = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error receiving message");

        // Make sure this is what we sent
        let header = res.message.session_header();
        assert_eq!(header.header_type, i32::from(SessionHeaderType::Reply));
        assert_eq!(header.session_id, session.id());

        // Let's trigger now a timeout
        // Send a message to the underlying gateway

        let msg = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            request_msg.as_bytes().to_vec(),
        );

        let mut session_message = SessionMessage::from(msg);
        session_message.info.id = session.id();

        // Send a message to the underlying gateway
        let res = session
            .on_message(session_message, MessageDirection::South)
            .await;

        assert!(res.is_ok(), "{}", res.unwrap_err());

        // Wait for 200ms to trigger the timeout
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Make sure we received an error
        let err = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect_err("error receiving message");

        // Make sure this is a timeout error
        match err {
            SessionError::Timeout {
                error: _error,
                message,
            } => {
                let blob = ProtoPublish::from(message.message)
                    .msg
                    .as_ref()
                    .expect("error getting message")
                    .blob
                    .clone();
                assert_eq!(blob, request_msg.as_bytes());
            }
            _ => panic!("unexpected error"),
        }
    }
}
