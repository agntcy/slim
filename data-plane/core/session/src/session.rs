// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use slim_datapath::{
    api::{ProtoMessage as Message, ProtoSessionMessageType},
    messages::Name,
};

use tokio::sync::mpsc::{self};
use tracing::debug;

use crate::{
    MessageDirection,
    common::SessionMessage,
    errors::SessionError,
    session_config::SessionConfig,
    session_receiver::SessionReceiver,
    session_sender::SessionSender,
    traits::{MessageHandler, ProcessingState},
    transmitter::SessionTransmitter,
};

pub(crate) struct Session {
    local_name: Name,
    sender: SessionSender,
    receiver: SessionReceiver,
    processing_state: ProcessingState,
}

impl Session {
    pub(crate) fn new(
        session_id: u32,
        session_config: SessionConfig,
        local_name: &Name,
        tx: SessionTransmitter,
        tx_signals: mpsc::Sender<SessionMessage>,
    ) -> Self {
        let timer_settings = if let Some(duration) = session_config.interval
            && let Some(max_retries) = session_config.max_retries
        {
            let timer_settings = crate::timer_factory::TimerSettings::constant(duration)
                .with_max_retries(max_retries);
            Some(timer_settings)
        } else {
            None
        };

        let sender = SessionSender::new(
            timer_settings.clone(),
            session_id,
            session_config.session_type,
            tx.clone(),
            Some(tx_signals.clone()),
        );
        let receiver = SessionReceiver::new(
            timer_settings,
            session_id,
            local_name.clone(),
            session_config.session_type,
            tx.clone(),
            Some(tx_signals.clone()),
        );

        Session {
            local_name: local_name.clone(),
            sender,
            receiver,
            processing_state: ProcessingState::Active,
        }
    }

    pub async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        match message {
            SessionMessage::OnMessage {
                message,
                direction,
                ack_tx,
            } => {
                debug!(
                    "received message {} type {:?} on {} from {} (direction {:?})",
                    message.get_id(),
                    message.get_session_message_type(),
                    self.local_name,
                    message.get_source(),
                    direction
                );
                self.on_application_message(message, direction, ack_tx)
                    .await
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_timeout(message_id, message_type, name).await,
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_failure(message_id, message_type, name).await,
            SessionMessage::StartDrain { grace_period: _ } => {
                self.processing_state = ProcessingState::Draining;
                self.sender.start_drain();
                self.receiver.start_drain();
                Ok(())
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message
            ))),
        }
    }

    pub async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        debug!("add participant {} on {}", endpoint, self.local_name);
        self.sender.add_endpoint(endpoint).await
    }

    pub fn remove_endpoint(&mut self, endpoint: &Name) {
        debug!("remove participant {} on {}", endpoint, self.local_name);
        self.sender.remove_endpoint(endpoint);
    }

    pub fn close(&mut self) {
        self.sender.close();
        self.receiver.close();
    }

    async fn on_application_message(
        &mut self,
        message: Message,
        direction: MessageDirection,
        ack_tx: Option<tokio::sync::oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::Msg => {
                if direction == MessageDirection::South {
                    // message from app to slim, give it to the sender with ack
                    self.sender.on_message(message, ack_tx).await
                } else {
                    // message from slim to the app, give it to the receiver
                    // Signal ack immediately for incoming messages
                    if let Some(tx) = ack_tx {
                        let _ = tx.send(Ok(()));
                    }
                    self.receiver.on_message(message).await
                }
            }
            ProtoSessionMessageType::MsgAck | ProtoSessionMessageType::RtxRequest => {
                self.sender.on_message(message, ack_tx).await
            }
            ProtoSessionMessageType::RtxReply => {
                // Signal ack immediately for control messages
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                self.receiver.on_message(message).await
            }
            _ => {
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                Err(SessionError::Processing(format!(
                    "Unexpected message type {:?}",
                    message.get_session_message_type()
                )))
            }
        }
    }

    async fn on_timer_timeout(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
    ) -> Result<(), SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => self.sender.on_timer_timeout(id).await,
            ProtoSessionMessageType::RtxRequest => {
                self.receiver.on_timer_timeout(id, name.unwrap()).await
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message_type
            ))),
        }
    }

    async fn on_timer_failure(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
    ) -> Result<(), SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => self.sender.on_timer_failure(id).await,
            ProtoSessionMessageType::RtxRequest => {
                self.receiver.on_timer_failure(id, name.unwrap()).await
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message_type
            ))),
        }
    }
}

/// Implementation of MessageHandler trait for Session
/// This allows Session to be used as a layer in the generic layer system
#[async_trait]
impl MessageHandler for Session {
    async fn init(&mut self) -> Result<(), SessionError> {
        // Session is the innermost layer, no initialization needed
        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        // Process through the session's existing on_message method
        // Session is the innermost layer, so it doesn't delegate to anything
        self.on_message(message).await
    }

    async fn add_endpoint(
        &mut self,
        endpoint: &slim_datapath::messages::Name,
    ) -> Result<(), SessionError> {
        self.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &slim_datapath::messages::Name) {
        self.remove_endpoint(endpoint);
    }

    fn needs_drain(&self) -> bool {
        !(self.sender.drain_completed() && self.receiver.drain_completed())
    }

    fn processing_state(&self) -> ProcessingState {
        self.processing_state
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        self.close();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_datapath::api::ProtoSessionType;
    use std::{collections::HashMap, time::Duration};
    use tokio::time::timeout;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_send_message() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim.clone(), tx_app.clone());
        let session_id = 10;

        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let session_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(200)),
            mls_enabled: false,
            initiator: false,
            metadata: HashMap::new(),
        };

        // create session
        let mut session = Session::new(
            session_id,
            session_config,
            &local_name,
            tx.clone(),
            tx_signal.clone(),
        );

        // Add the remote endpoint to the session sender
        session
            .add_endpoint(&remote_name)
            .await
            .expect("error adding participant");

        // Create a test message from app to slim (south direction)
        let mut message = Message::builder()
            .source(local_name.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message.set_session_message_type(ProtoSessionMessageType::Msg);

        // Send the message
        session
            .on_message(SessionMessage::OnMessage {
                message: message.clone(),
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .await
            .expect("error sending message");

        // Check that the message is received on slim
        let received = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error in received message");

        // update the message and check that it was received correctly
        message.set_message_id(1);
        message.set_session_type(ProtoSessionType::PointToPoint);
        message.get_session_header_mut().set_session_id(session_id);
        assert_eq!(received, message);

        // check that a timer is triggered and is received on rx_signals
        let timer_signal = timeout(Duration::from_millis(300), rx_signal.recv())
            .await
            .expect("timeout waiting for timer signal")
            .expect("channel closed");

        // Verify it's a TimerTimeout signal
        match &timer_signal {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                ..
            } => {
                assert_eq!(*message_id, 1);
                assert_eq!(*message_type, ProtoSessionMessageType::Msg);
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", timer_signal),
        }

        // send the timeout message to the session
        session
            .on_message(timer_signal)
            .await
            .expect("error handling timer timeout");

        // check that the message is sent again correctly to slim
        let retransmitted = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for retransmitted message")
            .expect("channel closed")
            .expect("error in retransmitted message");

        // Verify the retransmitted message
        assert_eq!(retransmitted, message);

        // get a message ack
        let mut ack_message = Message::builder()
            .source(remote_name.clone())
            .destination(local_name.clone())
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        ack_message.set_session_message_type(ProtoSessionMessageType::MsgAck);
        ack_message.get_session_header_mut().set_message_id(1);
        ack_message
            .get_session_header_mut()
            .set_session_id(session_id);
        ack_message.get_slim_header_mut().set_incoming_conn(Some(1));

        // Send the ack to the session
        session
            .on_message(SessionMessage::OnMessage {
                message: ack_message,
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error sending ack");

        // After receiving the ack, no more timer signals should be sent
        // Wait a bit to ensure no timer signal arrives
        let no_timer = timeout(Duration::from_millis(300), rx_signal.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after ack, but got one"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_receive_message() {
        // Create the session
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);

        let tx = SessionTransmitter::new(tx_slim.clone(), tx_app.clone());
        let session_id = 10;

        let local_name = Name::from_strings(["org", "ns", "local"]);
        let remote_name = Name::from_strings(["org", "ns", "remote"]);

        let session_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(200)),
            mls_enabled: false,
            initiator: false,
            metadata: HashMap::new(),
        };

        let mut session = Session::new(
            session_id,
            session_config,
            &local_name,
            tx.clone(),
            tx_signal.clone(),
        );

        // Receive message 1 from slim
        let mut message1 = Message::builder()
            .source(local_name.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload_1", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(ProtoSessionMessageType::Msg);
        message1.get_session_header_mut().set_message_id(1);
        message1.get_session_header_mut().set_session_id(session_id);
        message1.get_slim_header_mut().set_incoming_conn(Some(1));

        session
            .on_message(SessionMessage::OnMessage {
                message: message1.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error receiving message1");

        // Check that the message goes to the application on rx_app
        let received1 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message1 on rx_app")
            .expect("channel closed")
            .expect("error in received message1");

        // Check that the message is the right one
        assert_eq!(received1, message1);

        // Check that an ack was sent to SLIM
        let ack1 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack1")
            .expect("channel closed")
            .expect("error in received ack1");

        // Check that the ack is correct
        assert_eq!(
            ack1.get_session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack1.get_session_header().get_message_id(), 1);
        assert_eq!(ack1.get_dst(), local_name);

        // receive message 3 from slim direction north (skipping message 2)
        let mut message3 = Message::builder()
            .source(local_name.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload_3", vec![7, 8, 9])
            .build_publish()
            .unwrap();
        message3.set_session_message_type(ProtoSessionMessageType::Msg);
        message3.get_session_header_mut().set_message_id(3);
        message3.get_session_header_mut().set_session_id(session_id);
        message3.get_slim_header_mut().set_incoming_conn(Some(1));

        session
            .on_message(SessionMessage::OnMessage {
                message: message3.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error receiving message3");

        // check that the message does not go to the app (it's buffered waiting for message 2)
        let no_message = timeout(Duration::from_millis(100), rx_app.recv()).await;
        assert!(
            no_message.is_err(),
            "Expected no message on rx_app, but got one"
        );

        // Check that an ack was sent for message 3
        let ack3 = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for ack3")
            .expect("channel closed")
            .expect("error in received ack3");

        assert_eq!(
            ack3.get_session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Wait for the RTX timer to trigger
        let timer_signal = timeout(Duration::from_millis(600), rx_signal.recv())
            .await
            .expect("timeout waiting for RTX timer signal")
            .expect("channel closed");

        // Verify it's a TimerTimeout signal for RTX
        match &timer_signal {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                ..
            } => {
                assert_eq!(*message_id, 2);
                assert_eq!(*message_type, ProtoSessionMessageType::RtxRequest);
                assert!(name.is_some());
            }
            _ => panic!("Expected TimerTimeout signal, got: {:?}", timer_signal),
        }

        // Send the timer signal to the session to trigger RTX request
        session
            .on_message(timer_signal)
            .await
            .expect("error handling RTX timer");

        // Verify that an RTX request is sent to SLIM
        let rtx_request = timeout(Duration::from_millis(100), rx_slim.recv())
            .await
            .expect("timeout waiting for RTX request")
            .expect("channel closed")
            .expect("error in RTX request");

        assert_eq!(
            rtx_request.get_session_message_type(),
            ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(rtx_request.get_session_header().get_message_id(), 2);
        assert_eq!(rtx_request.get_dst(), local_name);

        // Create message 2 and send it as an RtxReply
        let mut message2 = Message::builder()
            .source(local_name.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload_2", vec![9, 10, 11, 12])
            .build_publish()
            .unwrap();
        message2.set_session_message_type(ProtoSessionMessageType::RtxReply);
        message2.get_session_header_mut().set_message_id(2);
        message2.get_session_header_mut().set_session_id(session_id);
        message2.get_slim_header_mut().set_incoming_conn(Some(1));

        session
            .on_message(SessionMessage::OnMessage {
                message: message2.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error receiving message2 as RtxReply");

        // Check that no other timeout is sent to rx_signal
        let no_timer = timeout(Duration::from_millis(300), rx_signal.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after receiving RTX reply, but got one"
        );

        // Check that both message 2 and 3 are delivered in order to the application
        // Message 2 should be delivered first
        let received2 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message2 on rx_app")
            .expect("channel closed")
            .expect("error in received message2");

        assert_eq!(received2.get_id(), 2);
        assert_eq!(received2.get_source(), local_name);

        // Message 3 should be delivered next (it was buffered)
        let received3 = timeout(Duration::from_millis(100), rx_app.recv())
            .await
            .expect("timeout waiting for message3 on rx_app")
            .expect("channel closed")
            .expect("error in received message3");

        assert_eq!(received3.get_id(), 3);
        assert_eq!(received3.get_source(), local_name);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end() {
        // Create two sessions, one will act as sender and one as receiver
        let session_id = 10;

        // Sender session setup
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(10);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal_sender, mut rx_signal_sender) = tokio::sync::mpsc::channel(10);

        let tx_sender = SessionTransmitter::new(tx_slim_sender.clone(), tx_app_sender.clone());

        let sender_name = Name::from_strings(["org", "ns", "sender"]);
        let receiver_name = Name::from_strings(["org", "ns", "receiver"]);

        let sender_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(200)),
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        };

        let mut sender_session = Session::new(
            session_id,
            sender_config,
            &sender_name,
            tx_sender.clone(),
            tx_signal_sender.clone(),
        );

        // Add receiver as endpoint for sender
        sender_session
            .add_endpoint(&receiver_name)
            .await
            .expect("error adding participant");

        // Receiver session setup
        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(10);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (tx_signal_receiver, _rx_signal_receiver) = tokio::sync::mpsc::channel(10);

        let tx_receiver =
            SessionTransmitter::new(tx_slim_receiver.clone(), tx_app_receiver.clone());

        let receiver_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(200)),
            mls_enabled: false,
            initiator: false,
            metadata: HashMap::new(),
        };

        let mut receiver_session = Session::new(
            session_id,
            receiver_config,
            &receiver_name,
            tx_receiver.clone(),
            tx_signal_receiver.clone(),
        );

        // Add sender as endpoint for receiver
        receiver_session
            .add_endpoint(&sender_name)
            .await
            .expect("error adding participant");

        // Send message 1 from the application
        let mut message1 = Message::builder()
            .source(sender_name.clone())
            .destination(receiver_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(ProtoSessionMessageType::Msg);

        sender_session
            .on_message(SessionMessage::OnMessage {
                message: message1.clone(),
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .await
            .expect("error sending message from sender");

        // Check that is received on the slim side
        let sent_message = timeout(Duration::from_millis(100), rx_slim_sender.recv())
            .await
            .expect("timeout waiting for message on sender slim channel")
            .expect("channel closed")
            .expect("error in sent message");

        assert_eq!(
            sent_message.get_session_message_type(),
            ProtoSessionMessageType::Msg
        );
        assert_eq!(sent_message.get_id(), 1);
        assert_eq!(sent_message.get_dst(), receiver_name);

        // Call the on message on the receiver side
        let mut received_message = sent_message.clone();
        received_message
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        receiver_session
            .on_message(SessionMessage::OnMessage {
                message: received_message.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error receiving message on receiver");

        // Check that an ack is received on slim
        let ack_message = timeout(Duration::from_millis(100), rx_slim_receiver.recv())
            .await
            .expect("timeout waiting for ack on receiver slim channel")
            .expect("channel closed")
            .expect("error in ack message");

        assert_eq!(
            ack_message.get_session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack_message.get_session_header().get_message_id(), 1);
        assert_eq!(ack_message.get_dst(), sender_name);

        // Check that the message is delivered to the app
        let app_message = timeout(Duration::from_millis(100), rx_app_receiver.recv())
            .await
            .expect("timeout waiting for message on receiver app channel")
            .expect("channel closed")
            .expect("error in app message");

        assert_eq!(app_message.get_id(), 1);
        assert_eq!(app_message.get_source(), sender_name);

        // Call the on message on the sender side with the ack
        let mut ack_to_sender = ack_message.clone();
        ack_to_sender
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        sender_session
            .on_message(SessionMessage::OnMessage {
                message: ack_to_sender,
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .await
            .expect("error processing ack on sender");

        // Wait to ensure the timer is stopped and no retransmission occurs
        let no_retransmit = timeout(Duration::from_millis(300), rx_slim_sender.recv()).await;
        assert!(
            no_retransmit.is_err(),
            "Expected no retransmission after ack, but got one"
        );

        // Also verify no timer signal is generated
        let no_timer = timeout(Duration::from_millis(100), rx_signal_sender.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after ack, but got one"
        );
    }
}
