// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use slim_auth::traits::{TokenProvider, Verifier};
use tracing::debug;

use crate::errors::SessionError;
use crate::fire_and_forget::FireAndForget;
use crate::session::{
    CommonSession, Id, MessageDirection, SessionConfigTrait, SessionDirection, State,
};
use crate::session::{MessageHandler, SessionConfig, SessionTransmitter};
use crate::{FireAndForgetConfiguration, SessionMessage, timer};
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::encoder::Agent;

/// Configuration for the Request Response session
/// This configuration is used to set the maximum number of retries and the timeout
#[derive(Debug, Clone, PartialEq)]
pub struct RequestResponseConfiguration {
    ff_conf: FireAndForgetConfiguration,
}

impl SessionConfigTrait for RequestResponseConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::RequestReply(config) => {
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
            ff_conf: FireAndForgetConfiguration {
                timeout: Some(std::time::Duration::from_millis(333)),
                max_retries: Some(1), // we only retry once
                sticky: false,        // we do not want to keep the session alive
            },
        }
    }
}

impl RequestResponseConfiguration {
    pub fn new(timeout: std::time::Duration, sticky: bool) -> Self {
        RequestResponseConfiguration {
            ff_conf: FireAndForgetConfiguration {
                timeout: Some(timeout / 3),
                max_retries: Some(1), // we only retry once
                sticky,               // we want to keep the session alive
            },
        }
    }

    pub fn timeout(&self) -> std::time::Duration {
        self.ff_conf.timeout.unwrap() * 3
    }

    pub fn sticky(&self) -> bool {
        self.ff_conf.sticky
    }
}

impl std::fmt::Display for RequestResponseConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RequestResponseConfiguration: {}",
            self.ff_conf
        )
    }
}

/// Internal state of the Request Response session
struct RequestResponseInternal<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    ff_session: FireAndForget<P, V, T>,
    timers: RwLock<HashMap<u32, (timer::Timer, SessionMessage)>>,
}

#[async_trait]
impl<P, V, T> timer::TimerObserver for RequestResponseInternal<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
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
            .ff_session
            .tx_ref()
            .send_to_app(Err(SessionError::Timeout {
                session_id: self.ff_session.id(),
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
pub(crate) struct RequestResponse<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    internal: Arc<RequestResponseInternal<P, V, T>>,
}

impl<P, V, T> RequestResponse<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: RequestResponseConfiguration,
        session_direction: SessionDirection,
        source: Agent,
        tx: T,
        identity_provider: P,
        identity_verifier: V,
        mls_enabled: bool,
    ) -> Self {
        debug!("creating new RequestResponse session with id: {}", id);

        // create the FireAndForget session
        let ff_session = FireAndForget::new(
            id,
            session_config.ff_conf,
            session_direction,
            source,
            tx,
            identity_provider.clone(),
            identity_verifier.clone(),
            mls_enabled,
        );

        // return the RequestResponse session
        RequestResponse {
            internal: Arc::new(RequestResponseInternal {
                ff_session,
                timers: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub(crate) async fn send_message_with_timer(
        &self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        // get message id
        let message_id = message.info.message_id.expect("message id not found");

        // get session config
        let session_config = match self.internal.ff_session.session_config() {
            SessionConfig::FireAndForget(config) => config,
            _ => {
                return Err(SessionError::AppTransmission(
                    "invalid session config".to_string(),
                ));
            }
        };

        // get duration from configuration
        let duration = session_config.timeout.unwrap() * 3;

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
            .ff_session
            .on_message(message.clone(), MessageDirection::South)
            .await?;

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
impl<P, V, T> CommonSession<P, V, T> for RequestResponse<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        // concat the token stream
        self.internal.ff_session.id()
    }

    fn state(&self) -> &State {
        self.internal.ff_session.state()
    }

    fn session_config(&self) -> SessionConfig {
        // Extract the FireAndForgetConfiguration from the FireAndForget session
        let ff_config = match self.internal.ff_session.session_config() {
            SessionConfig::FireAndForget(config) => config.clone(),
            _ => {
                panic!("RequestResponse session expected FireAndForget configuration");
            }
        };

        SessionConfig::RequestReply(RequestResponseConfiguration {
            ff_conf: ff_config,
        })
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        // Set the session configuration for the FireAndForget session
        match session_config {
            SessionConfig::RequestReply(config) => {
                self.internal
                    .ff_session
                    .set_session_config(&SessionConfig::FireAndForget(config.ff_conf.clone()))?;
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected RequestResponse, got {:?}",
                session_config
            ))),
        }
    }

    fn source(&self) -> &Agent {
        self.internal.ff_session.source()
    }

    fn identity_provider(&self) -> P {
        self.internal.ff_session.identity_provider().clone()
    }

    fn identity_verifier(&self) -> V {
        self.internal.ff_session.identity_verifier().clone()
    }

    fn tx(&self) -> T {
        self.internal.ff_session.tx().clone()
    }

    fn tx_ref(&self) -> &T {
        self.internal.ff_session.tx_ref()
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for RequestResponse<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
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
                // Get the message type
                let session_message_type = message.info.get_session_message_type();

                match session_message_type {
                    ProtoSessionMessageType::Reply => {
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
                    ProtoSessionMessageType::Request => {
                        // this is a request - set the session_type of the session
                        // info to reply to allow the app to reply using this session info
                        message
                            .info
                            .set_session_message_type(ProtoSessionMessageType::Reply);
                    }
                    _ => {
                        // we just send the message up to the fire&forget session
                        // if they need to be dropped, they will be dropped there
                    }
                }

                // Forward message to underlying fire and forget session
                // This will handle the message and send it to the app
                self.internal
                    .ff_session
                    .on_message(message, direction)
                    .await
            }
            MessageDirection::South => {
                // Get the message type
                let session_message_type = message.info.get_session_message_type();

                // we are sending the message over slim.
                // Let's start setting the session header
                session_header.session_id = self.internal.ff_session.id();
                message.info.id = self.internal.ff_session.id();

                match session_message_type {
                    ProtoSessionMessageType::Reply => {
                        // this is a reply - make sure the message_id matches the request
                        message.info.message_id.ok_or_else(|| {
                            SessionError::SlimTransmission(
                                "message id not set for reply".to_string(),
                            )
                        })?;

                        message
                            .info
                            .set_session_message_type(ProtoSessionMessageType::Reply);
                        message
                            .info
                            .set_session_type(ProtoSessionType::SessionRequestReply);

                        self.internal
                            .ff_session
                            .on_message(message, direction)
                            .await
                    }
                    _ => {
                        // In this case, we are sending a request
                        // set the message id to something random
                        session_header.set_message_id(rand::random::<u32>());
                        session_header.set_session_message_type(ProtoSessionMessageType::Request);
                        session_header.set_session_type(ProtoSessionType::SessionRequestReply);

                        message.info.set_message_id(session_header.message_id);

                        self.send_message_with_timer(message).await
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::MockTransmitter;
    use slim_auth::simple::SimpleGroup;
    use slim_datapath::{
        api::{ProtoMessage, ProtoPublish},
        messages::{Agent, AgentType},
    };

    #[tokio::test]
    async fn test_rr_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let session_config =
            RequestResponseConfiguration::new(std::time::Duration::from_secs(5), false);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = RequestResponse::new(
            0,
            session_config.clone(),
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SimpleGroup::new("a", "group"),
            SimpleGroup::new("a", "group"),
            false,
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::RequestReply(session_config)
        );
    }

    #[tokio::test]
    async fn test_request_response_on_message() {
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let session_config =
            RequestResponseConfiguration::new(std::time::Duration::from_secs(5), false);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = RequestResponse::new(
            0,
            session_config,
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SimpleGroup::new("a", "group"),
            SimpleGroup::new("a", "group"),
            false,
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
        header.set_session_message_type(ProtoSessionMessageType::Request);

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

        let tx = MockTransmitter { tx_app, tx_slim };

        let session_config =
            RequestResponseConfiguration::new(std::time::Duration::from_secs(5), false);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        {
            let _session = RequestResponse::new(
                0,
                session_config,
                SessionDirection::Bidirectional,
                source.clone(),
                tx,
                SimpleGroup::new("a", "group"),
                SimpleGroup::new("a", "group"),
                false,
            );
        }
    }
}
