// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use core::fmt;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rand::Rng;
use tracing::{debug, warn};

use crate::errors::SessionError;
use crate::session::{AppChannelSender, GwChannelSender, SessionConfig};
use crate::session::{
    Common, CommonSession, Id, Info, MessageDirection, Session, SessionDirection, State,
};
use crate::timer;
use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
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
    timers: RwLock<HashMap<u32, (timer::Timer, Message)>>,
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
                message,
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
        message: Message,
        message_id: u32,
    ) -> Result<(), SessionError> {
        // create new timer
        let timer = timer::Timer::new(message_id, 1000, 0);

        // send message
        self.internal
            .common
            .tx_gw_ref()
            .send(Ok(message.clone()))
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
        mut message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // set the session type
        let header = utils::get_session_header_as_mut(&mut message);
        if header.is_none() {
            return Err(SessionError::AppTransmission("missing header".to_string()));
        }

        let header = header.unwrap();

        // clone tx
        match direction {
            MessageDirection::North => {
                // message for the app - check if request or response
                let session_type = match utils::int_to_service_type(header.header_type) {
                    Some(t) => t,
                    None => Err(SessionError::AppTransmission(
                        "unknown session type".to_string(),
                    ))?,
                };

                match session_type {
                    SessionHeaderType::Reply => {
                        // this is a reply - remove the timer
                        let message_id = header.message_id;
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
                        // this is a request - send it to app
                    }
                    _ => Err(SessionError::AppTransmission(format!(
                        "request/reply session: unsupported session type: {:?}",
                        session_type
                    )))?,
                }

                // create info
                let info = Info::new(self.id(), self.state().clone());

                self.internal
                    .common
                    .tx_app_ref()
                    .send(Ok((message, info)))
                    .await
                    .map_err(|e| SessionError::AppTransmission(e.to_string()))
            }
            MessageDirection::South => {
                // Send message with timer
                let message_id = rand::rng().random();
                header.message_id = message_id;

                self.send_message_with_timer(message, message_id).await
            }
        }
    }
}

delegate_common_behavior!(RequestResponse, internal, common);

#[cfg(test)]
mod tests {
    use super::*;
    use agp_datapath::messages::encoder;

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
    async fn test_fire_and_forget_on_message() {
        let (tx_gw, _rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_config = RequestResponseConfiguration {
            max_retries: 0,
            timeout: std::time::Duration::from_millis(1000),
        };

        let session = RequestResponse::new(
            0,
            session_config,
            SessionDirection::Bidirectional,
            tx_gw,
            tx_app,
        );

        let payload = vec![0x1, 0x2, 0x3, 0x4];

        let mut msg = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            payload.clone(),
        );

        // set the session id in the message
        let header = utils::get_session_header_as_mut(&mut msg).unwrap();
        header.session_id = 1;

        // Send a message to the underlying gateway
        let res = session
            .on_message(msg.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        // we will wait for a response, but as no one is reply, we will get a timeout
        let res = rx_app.recv().await.expect("no message received");
        assert!(res.is_err());

        // We also should get the message back in the error
        let err = res.unwrap_err();
        match err {
            SessionError::Timeout {
                error: _error,
                message,
            } => {
                let blob = utils::get_message_as_publish(&message)
                    .expect("error getting message")
                    .msg
                    .as_ref()
                    .expect("error getting message")
                    .blob
                    .clone();
                assert_eq!(blob, payload);
            }
            _ => panic!("unexpected error"),
        }
    }
}
