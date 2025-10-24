// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use parking_lot::{Mutex, RwLock};
use slim_mls::mls::Mls;
use std::{collections::HashMap, sync::Arc, time::Duration};

// Third-party crates
use async_trait::async_trait;
use tokio::{
    sync::mpsc,
    time::{self, Instant},
};
use tracing::{debug, error, trace, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    Status,
    api::{
        ApplicationPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType,
        SessionHeader, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    Common, CommonSession, Id, MessageDirection, MessageHandler, SessionConfig, SessionConfigTrait,
    State, Transmitter,
    common::SessionMessage,
    errors::SessionError,
    mls_state::{MlsEndpoint, MlsState},
    producer_buffer, receiver_buffer,
    session_controller::{SessionController, SessionModerator, SessionParticipant},
    session_receiver::SessionReceiver,
    session_sender::SessionSender,
    timer,
    timer_factory::TimerSettings,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

// this must be a number > 1
const STREAM_BROADCAST: u32 = 50;

/// Configuration for the Multicast session
#[derive(Debug, Clone, PartialEq)]
pub struct MulticastConfiguration {
    pub channel_name: Name,
    pub max_retries: u32,
    pub timeout: std::time::Duration,
    pub mls_enabled: bool,
    pub(crate) initiator: bool,
    pub metadata: HashMap<String, String>,
}

impl SessionConfigTrait for MulticastConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::Multicast(config) => {
                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected Multicast, got {:?}",
                session_config
            ))),
        }
    }
}

impl Default for MulticastConfiguration {
    fn default() -> Self {
        MulticastConfiguration {
            channel_name: Name::from_strings(["agntcy", "ns", "multicast"]),
            max_retries: 10,
            timeout: std::time::Duration::from_millis(1000),
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        }
    }
}

impl std::fmt::Display for MulticastConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MulticastConfiguration: channel_name: {}, initiator: {}, max_retries: {}, timeout: {} ms",
            self.channel_name,
            self.initiator,
            self.max_retries,
            self.timeout.as_millis(),
        )
    }
}

impl MulticastConfiguration {
    pub fn new(
        channel_name: Name,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
        mls_enabled: bool,
        metadata: HashMap<String, String>,
    ) -> Self {
        MulticastConfiguration {
            channel_name,
            max_retries: max_retries.unwrap_or(0),
            timeout: timeout.unwrap_or(std::time::Duration::from_millis(0)),
            mls_enabled,
            initiator: true,
            metadata,
        }
    }

    #[cfg(test)]
    pub fn new_with_initiator(
        channel_name: Name,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
        mls_enabled: bool,
        initiator: bool,
        metadata: HashMap<String, String>,
    ) -> Self {
        Self {
            channel_name,
            max_retries: max_retries.unwrap_or(0),
            timeout: timeout.unwrap_or(std::time::Duration::from_millis(0)),
            mls_enabled,
            initiator,
            metadata,
        }
    }
}

pub(crate) struct Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    common: Common<P, V, T>,
    sender: SessionSender<T>,
    receiver: SessionReceiver<T>,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

impl<P, V, T> Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: MulticastConfiguration,
        name: Name,
        tx_slim_app: T,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        sender: SessionSender<T>,
        receiver: SessionReceiver<T>,
        tx_session: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(128);

        let common = Common::new(
            id,
            SessionConfig::Multicast(session_config.clone()),
            name.clone(),
            tx_slim_app.clone(),
            identity_provider,
            identity_verifier,
            session_config.mls_enabled,
            storage_path,
        );

        common.set_dst(session_config.channel_name);

        let session = Multicast {
            common,
            sender,
            receiver,
            tx,
        };
        session
    }

    pub fn with_dst<R>(&self, f: impl FnOnce(Option<&Name>) -> R) -> R {
        self.common.with_dst(f)
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &mut self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::Msg => {
                if direction == MessageDirection::South {
                    // message from app to slim, give it to the sender
                    self.sender.on_message(message).await.map(|_| ())
                } else {
                    // message from slim to the app, give it to the receiver
                    self.receiver.on_message(message).await.map(|_| ())
                }
            }
            ProtoSessionMessageType::MsgAck | ProtoSessionMessageType::RtxRequest => {
                self.sender.on_message(message).await.map(|_| ())
            }
            ProtoSessionMessageType::RtxReply => {
                self.receiver.on_message(message).await.map(|_| ())
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message.get_session_message_type()
            ))),
        }
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        // concat the token stream
        self.common.id()
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_config(&self) -> SessionConfig {
        self.common.session_config()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        self.common.set_session_config(session_config)
    }

    fn source(&self) -> &Name {
        self.common.source()
    }

    fn dst(&self) -> Option<Name> {
        // multicast sessions do not use dst
        self.common.dst()
    }

    fn dst_arc(&self) -> Arc<RwLock<Option<Name>>> {
        self.common.dst_arc()
    }

    fn identity_provider(&self) -> P {
        self.common.identity_provider().clone()
    }

    fn identity_verifier(&self) -> V {
        self.common.identity_verifier().clone()
    }

    fn tx(&self) -> T {
        self.common.tx().clone()
    }

    fn tx_ref(&self) -> &T {
        self.common.tx_ref()
    }

    fn set_dst(&self, dst: Name) {
        // allow setting on the common even if unused
        self.common.set_dst(dst)
    }

    fn mls(&self) -> Option<Arc<Mutex<Mls<P, V>>>> {
        self.common.mls()
    }
}

/*#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use tokio::time;
    use tracing_test::traced_test;

    use slim_datapath::messages::Name;

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let tx = SessionTransmitter {
            slim_tx: tx_slim.clone(),
            app_tx: tx_app.clone(),
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let source = Name::from_strings(["agntcy", "ns", "local"]).with_id(0);
        let stream = Name::from_strings(["agntcy", "ns", "local_stream"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream.clone(),
            None,
            None,
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            0,
            session_config.clone(),
            source.clone(),
            tx.clone(),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Multicast(session_config.clone())
        );

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream,
            Some(10),
            Some(Duration::from_millis(1000)),
            false,
            false,
            HashMap::new(),
        );

        let session = Multicast::new(
            1,
            session_config.clone(),
            source.clone(),
            tx,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        assert_eq!(session.id(), 1);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Multicast(session_config)
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_sender_and_receiver() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let tx_sender = SessionTransmitter {
            slim_tx: tx_slim_sender,
            app_tx: tx_app_sender,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let (tx_slim_receiver, _rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let tx_receiver = SessionTransmitter {
            slim_tx: tx_slim_receiver,
            app_tx: tx_app_receiver,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let send = Name::from_strings(["cisco", "default", "sender"]).with_id(0);
        let recv = Name::from_strings(["cisco", "default", "receiver"]).with_id(0);

        let session_config_sender: MulticastConfiguration =
            MulticastConfiguration::new_with_initiator(
                send.clone(),
                None,
                None,
                false,
                false,
                HashMap::new(),
            );
        let session_config_receiver: MulticastConfiguration = MulticastConfiguration::new(
            send.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let sender = Multicast::new(
            0,
            session_config_sender,
            send.clone(),
            tx_sender,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session_sender"),
            tx_session.clone(),
        );
        let receiver = Multicast::new(
            0,
            session_config_receiver,
            recv.clone(),
            tx_receiver,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session_receiver"),
            tx_session.clone(),
        );

        let payload = Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content());

        let mut message = Message::new_publish(
            &send,
            &recv,
            None,
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            payload,
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;

        // set session header type for test check
        let mut expected_msg = message.clone();
        expected_msg.set_session_message_type(ProtoSessionMessageType::Msg);
        expected_msg.set_session_type(ProtoSessionType::Multicast);
        expected_msg.set_fanout(STREAM_BROADCAST);

        // send a message from the sender app to the slim
        let res = sender
            .on_message(message.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        let msg = rx_slim_sender.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);

        // send the same message to the receiver
        let res = receiver
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);
        assert_eq!(msg.get_session_header().get_session_id(), 0);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_rtx_timeouts() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let sender = Name::from_strings(["agntcy", "ns", "sender"]).with_id(0);
        let receiver = Name::from_strings(["agntcy", "ns", "receiver"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            sender.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            0,
            session_config,
            sender.clone(),
            tx,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        let payload = Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content());
        let mut message = Message::new_publish(
            &sender,
            &receiver,
            None,
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            payload,
        );

        // set the session type
        let header = message.get_session_header_mut();
        header.set_session_message_type(ProtoSessionMessageType::Msg);
        header.set_session_id(0);

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app.recv().await.unwrap().unwrap();
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_session_id(), 0);

        // set msg id = 2 this will trigger a loss detection
        let header = message.get_session_header_mut();
        header.message_id = 2;

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // read rtxs from the slim channel, the original one + 5 retries
        for _ in 0..6 {
            let rtx_msg = rx_slim.recv().await.unwrap().unwrap();
            let rtx_header = rtx_msg.get_session_header();
            assert_eq!(rtx_header.session_id, 0);
            assert_eq!(rtx_header.message_id, 1);
            assert_eq!(
                rtx_header.session_message_type(),
                ProtoSessionMessageType::RtxRequest,
            );
        }

        time::sleep(Duration::from_millis(1000)).await;

        let expected_msg = "packet 1 lost, not retries left";
        assert!(logs_contain(expected_msg));
        let expected_msg = "a message was definitely lost in session 0";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_rtx_reception() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(8);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(8);

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let receiver = Name::from_strings(["agntcy", "ns", "receiver"]).with_id(0);
        let sender = Name::from_strings(["agntcy", "ns", "sender"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            sender.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            120,
            session_config,
            receiver.clone(),
            tx,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        let payload = Some(ApplicationPayload::new("", vec![0x1, 0x2, 0x3, 0x4]).as_content());
        let mut message = Message::new_publish(&sender, &receiver, None, None, payload);

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 120;

        // send 3 messages
        for _ in 0..3 {
            let res = session
                .on_message(message.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the slim channel
        for i in 0..3 {
            let msg = rx_slim.recv().await.unwrap().unwrap();
            let msg_header = msg.get_session_header();
            assert_eq!(msg_header.session_id, 120);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(
                msg_header.session_message_type(),
                ProtoSessionMessageType::Msg
            );
        }

        let slim_header = Some(SlimHeader::new(
            &sender,
            &receiver,
            "a",
            Some(
                SlimHeaderFlags::default()
                    .with_forward_to(0)
                    .with_incoming_conn(123),
            ), // set incoming conn, as it is required for the rtx
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::Multicast.into(),
            ProtoSessionMessageType::RtxRequest.into(),
            1,
            2,
        ));

        let payload = Some(ApplicationPayload::new("", vec![]).as_content());
        // receive an RTX for message 2
        let rtx = Message::new_publish_with_headers(slim_header, session_header, payload);

        // send the RTX from the slim
        let res = session
            .on_message(rtx.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // get rtx reply message from slim
        let msg = rx_slim.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 120);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply
        );
        assert_eq!(
            msg.get_payload().unwrap().as_application_payload().blob,
            vec![0x1, 0x2, 0x3, 0x4]
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_e2e_with_losses() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let tx_sender = SessionTransmitter {
            slim_tx: tx_slim_sender,
            app_tx: tx_app_sender,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let tx_receiver = SessionTransmitter {
            slim_tx: tx_slim_receiver,
            app_tx: tx_app_receiver,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let send = Name::from_strings(["cisco", "default", "sender"]).with_id(0);
        let recv = Name::from_strings(["cisco", "default", "receiver"]).with_id(0);

        let session_config_sender: MulticastConfiguration =
            MulticastConfiguration::new_with_initiator(
                recv.clone(),
                None,
                None,
                false,
                false,
                HashMap::new(),
            );
        let session_config_receiver: MulticastConfiguration = MulticastConfiguration::new(
            recv.clone(),
            Some(5),
            Some(Duration::from_millis(100)), // keep the timer shorter with respect to the beacon one
            // otherwise we don't know which message will be received first
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let sender = Multicast::new(
            0,
            session_config_sender,
            send.clone(),
            tx_sender,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session_sender"),
            tx_session.clone(),
        );
        let receiver = Multicast::new(
            0,
            session_config_receiver,
            recv.clone(),
            tx_receiver,
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session_receiver"),
            tx_session.clone(),
        );

        let payload = Some(ApplicationPayload::new("", vec![]).as_content());
        let mut message = Message::new_publish(
            &send,
            &recv,
            None,
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            payload,
        );
        message.set_incoming_conn(Some(0));
        message.get_session_header_mut().set_session_id(0);

        // send 3 messages from the producer app
        // send 3 messages
        for _ in 0..3 {
            let res = sender
                .on_message(message.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the sender slim channel
        // forward message 1 and 3 to the receiver
        for i in 0..3 {
            let mut msg = rx_slim_sender.recv().await.unwrap().unwrap();
            let msg_header = msg.get_session_header();
            assert_eq!(msg_header.session_id, 0);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(
                msg_header.session_message_type(),
                ProtoSessionMessageType::Msg
            );

            // the receiver should detect a loss for packet 1
            if i != 1 {
                // make sure to set the incoming connection to avoid panic
                msg.set_incoming_conn(Some(0));
                msg.get_session_header_mut().set_session_id(0);
                let res = receiver
                    .on_message(msg.clone(), MessageDirection::North)
                    .await;
                assert!(res.is_ok());
            }
        }

        // the receiver app should get the packet 0
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 0);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::Msg
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        // get the RTX from packet 1 and drop the first one before send it to sender
        let msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxRequest,
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );

        let mut msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );

        // send the second reply to the producer
        // make sure to set the incoming connection to avoid paninc
        msg.set_incoming_conn(Some(0));
        msg.get_session_header_mut().set_session_id(0);
        let res = sender
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // this should generate an RTX reply
        let mut msg = rx_slim_sender.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        // make sure to set the incoming connection to avoid paninc
        msg.set_incoming_conn(Some(0));
        let res = receiver
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the receiver app should get the packet 1 and 2, packet 1 is an RTX
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply,
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::Msg
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let source = Name::from_strings(["agntcy", "ns", "local"]).with_id(0);
        let stream = Name::from_strings(["agntcy", "ns", "stream"]);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream,
            None,
            None,
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        {
            let _session = Multicast::new(
                0,
                session_config.clone(),
                source.clone(),
                tx,
                SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
                SharedSecret::new("a", slim_auth::testutils::TEST_VALID_SECRET),
                std::path::PathBuf::from("/tmp/test_session"),
                tx_session.clone(),
            );
        }

        // session should be deleted, make sure the process loop is also closed
        time::sleep(Duration::from_millis(100)).await;

        // check that the session is deleted, by checking the log
        assert!(logs_contain(
            "stopping message processing on multicast session 0"
        ));
    }
}*/
