// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::api::{
    EncodedName, Participant, ProtoMessage as Message, ProtoName, ProtoSessionMessageType,
};

use tokio::sync::mpsc::{self};
use tracing::debug;

use crate::{
    Direction, MessageDirection,
    common::{SessionMessage, SessionOutput},
    errors::SessionError,
    session_config::SessionConfig,
    session_receiver::SessionReceiver,
    session_sender::SessionSender,
    traits::{MessageHandler, ProcessingState},
};

pub(crate) struct Session {
    local_name: ProtoName,
    sender: Option<SessionSender>,
    receiver: Option<SessionReceiver>,
    processing_state: ProcessingState,
}

impl Session {
    pub(crate) fn new(
        session_id: u32,
        session_config: SessionConfig,
        local_name: &ProtoName,
        tx_signals: mpsc::Sender<SessionMessage>,
        direction: Direction,
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

        let (shutdown_send, shutdown_receive) = direction.to_flags();

        let sender = if shutdown_send {
            None
        } else {
            Some(SessionSender::new(
                timer_settings.clone(),
                session_id,
                session_config.session_type,
                Some(tx_signals.clone()),
            ))
        };

        let receiver = if shutdown_receive {
            None
        } else {
            Some(SessionReceiver::new(
                timer_settings,
                session_id,
                local_name.clone(),
                session_config.session_type,
                Some(tx_signals.clone()),
            ))
        };

        Session {
            local_name: local_name.clone(),
            sender,
            receiver,
            processing_state: ProcessingState::Active,
        }
    }

    pub fn on_message(&mut self, message: SessionMessage) -> Result<SessionOutput, SessionError> {
        match message {
            SessionMessage::OnMessage {
                message,
                direction,
                ack_tx,
            } => {
                debug!(
                    message_id = %message.get_id(),
                    session_message_type = ?message.get_session_message_type(),
                    name = %self.local_name,
                    source = %message.get_source(),
                    direction = ?direction,
                    "received message",
                );
                self.on_application_message(message, direction, ack_tx)
            }
            SessionMessage::MessageError { error } => {
                debug!(
                    name = %self.local_name,
                    "received message error: {:?}", error,
                );

                match self.sender.as_mut() {
                    Some(sender) => sender.on_slim_failure(error),
                    None => Err(SessionError::SessionSenderShutdown),
                }
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_timeout(message_id, message_type, name),
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_failure(message_id, message_type, name),
            SessionMessage::StartDrain { grace_period: _ } => {
                self.processing_state = ProcessingState::Draining;
                if let Some(sender) = self.sender.as_mut() {
                    sender.start_drain();
                }
                if let Some(receiver) = self.receiver.as_mut() {
                    receiver.start_drain();
                }
                Ok(SessionOutput::new())
            }
            _ => Err(SessionError::SessionMessageInternalUnexpected(Box::new(
                message,
            ))),
        }
    }

    pub fn add_endpoint(&mut self, endpoint: &Participant) -> Result<SessionOutput, SessionError> {
        let name = endpoint.get_name()?;
        debug!(%name, local_name = %self.local_name, "add participant");
        if let Some(sender) = self.sender.as_mut() {
            sender.add_endpoint(endpoint)
        } else {
            Ok(SessionOutput::new())
        }
    }

    pub fn remove_endpoint(&mut self, endpoint: &ProtoName) {
        debug!(%endpoint, local_name = %self.local_name, "remove participant");
        if let Some(sender) = self.sender.as_mut() {
            sender.remove_endpoint(endpoint)
        }
        if let Some(receiver) = self.receiver.as_mut() {
            receiver.remove_endpoint(endpoint);
        }
    }

    pub fn close(&mut self) {
        if let Some(sender) = self.sender.as_mut() {
            sender.close();
        }
        if let Some(receiver) = self.receiver.as_mut() {
            receiver.close();
        }
    }

    fn on_application_message(
        &mut self,
        message: Message,
        direction: MessageDirection,
        ack_tx: Option<tokio::sync::oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<SessionOutput, SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::Msg => {
                if direction == MessageDirection::South {
                    match self.sender.as_mut() {
                        Some(sender) => sender.on_message(message, ack_tx),
                        None => {
                            let error = SessionError::SessionSenderShutdown;
                            if let Some(tx) = ack_tx {
                                let _ = tx.send(Err(error));
                            }
                            Err(SessionError::SessionSenderShutdown)
                        }
                    }
                } else {
                    match self.receiver.as_mut() {
                        Some(receiver) => receiver.on_message(message),
                        None => Err(SessionError::SessionReceiverShutdown),
                    }
                }
            }
            ProtoSessionMessageType::MsgAck | ProtoSessionMessageType::RtxRequest => {
                match self.sender.as_mut() {
                    Some(sender) => sender.on_message(message, ack_tx),
                    None => {
                        let error = SessionError::SessionSenderShutdown;
                        if let Some(tx) = ack_tx {
                            let _ = tx.send(Err(error));
                        }
                        Err(SessionError::SessionSenderShutdown)
                    }
                }
            }
            ProtoSessionMessageType::RtxReply => match self.receiver.as_mut() {
                Some(receiver) => receiver.on_message(message),
                None => Err(SessionError::SessionReceiverShutdown),
            },
            _ => {
                if let Some(tx) = ack_tx {
                    let _ = tx.send(Ok(()));
                }
                Err(SessionError::SessionMessageTypeUnexpected(
                    message.get_session_message_type(),
                ))
            }
        }
    }

    fn on_timer_timeout(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<EncodedName>,
    ) -> Result<SessionOutput, SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => match self.sender.as_mut() {
                Some(sender) => sender.on_timer_timeout(id),
                None => Err(SessionError::SessionSenderShutdown),
            },
            ProtoSessionMessageType::RtxRequest => {
                if let Some(name) = name {
                    match self.receiver.as_mut() {
                        Some(receiver) => receiver.on_timer_timeout(id, name),
                        None => Err(SessionError::SessionReceiverShutdown),
                    }
                } else {
                    Err(SessionError::MissingParticipantNameOnTimer)
                }
            }
            _ => Err(SessionError::SessionMessageTypeUnexpected(message_type)),
        }
    }

    fn on_timer_failure(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<EncodedName>,
    ) -> Result<SessionOutput, SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => match self.sender.as_mut() {
                Some(sender) => sender.on_timer_failure(id),
                None => Err(SessionError::SessionSenderShutdown),
            },
            ProtoSessionMessageType::RtxRequest => {
                if let Some(name) = name {
                    match self.receiver.as_mut() {
                        Some(receiver) => receiver.on_timer_failure(id, name),
                        None => Err(SessionError::SessionReceiverShutdown),
                    }
                } else {
                    Err(SessionError::MissingParticipantNameOnTimer)
                }
            }
            _ => Err(SessionError::SessionMessageTypeUnexpected(message_type)),
        }
    }
}

/// Implementation of MessageHandler trait for Session
/// This allows Session to be used as a layer in the generic layer system
impl MessageHandler for Session {
    async fn init(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<SessionOutput, SessionError> {
        Session::on_message(self, message)
    }

    async fn add_endpoint(
        &mut self,
        endpoint: &Participant,
    ) -> Result<SessionOutput, SessionError> {
        Session::add_endpoint(self, endpoint)
    }

    fn remove_endpoint(&mut self, endpoint: &slim_datapath::api::ProtoName) {
        Session::remove_endpoint(self, endpoint);
    }

    fn needs_drain(&self) -> bool {
        let sender_done = self.sender.as_ref().is_none_or(|s| s.drain_completed());
        let receiver_done = self.receiver.as_ref().is_none_or(|r| r.drain_completed());
        !(sender_done && receiver_done)
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
    use crate::common::OutboundMessage;

    use super::*;
    use slim_datapath::api::{ParticipantSettings, ProtoSessionType};
    use std::{collections::HashMap, time::Duration};
    use tokio::time::timeout;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_send_message() {
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);
        let session_id = 10;

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

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
            tx_signal.clone(),
            Direction::Bidirectional,
        );

        // Add the remote endpoint
        let output = session
            .add_endpoint(&Participant::new(
                remote_name.clone(),
                ParticipantSettings::receive_only(),
            ))
            .expect("error adding participant");
        assert!(output.messages.is_empty());

        // Create a test message from app to slim (south direction)
        let mut message = Message::builder()
            .source(local_name.clone())
            .destination(remote_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message.set_session_message_type(ProtoSessionMessageType::Msg);

        // Send the message
        let output = session
            .on_message(SessionMessage::OnMessage {
                message: message.clone(),
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .expect("error sending message");

        // Check that the output contains a ToSlim message
        assert_eq!(output.messages.len(), 1);
        let sent_msg = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m.clone(),
            _ => panic!("Expected ToSlim message"),
        };

        // Update the message and check it was sent correctly
        message.set_message_id(1);
        message.set_session_type(ProtoSessionType::PointToPoint);
        message.get_session_header_mut().set_session_id(session_id);
        assert_eq!(sent_msg, message);

        // Check that a timer is triggered and received on rx_signals
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

        // Send the timeout message to the session
        let output = session
            .on_message(timer_signal)
            .expect("error handling timer timeout");

        // Check that the message is sent again correctly
        assert_eq!(output.messages.len(), 1);
        let retransmitted = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m.clone(),
            _ => panic!("Expected ToSlim message on retransmit"),
        };
        assert_eq!(retransmitted, message);

        // Get a message ack
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
        let output = session
            .on_message(SessionMessage::OnMessage {
                message: ack_message,
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error sending ack");
        assert!(output.messages.is_empty());

        // After receiving the ack, no more timer signals should be sent
        let no_timer = timeout(Duration::from_millis(300), rx_signal.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after ack, but got one"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_receive_message() {
        let (tx_signal, mut rx_signal) = tokio::sync::mpsc::channel(10);
        let session_id = 10;

        let local_name = ProtoName::from_strings(["org", "ns", "local"]);
        let remote_name = ProtoName::from_strings(["org", "ns", "remote"]);

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
            tx_signal.clone(),
            Direction::Bidirectional,
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

        let output = session
            .on_message(SessionMessage::OnMessage {
                message: message1.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error receiving message1");

        // Should produce: ToApp(message1) + ToSlim(ack)
        assert_eq!(output.messages.len(), 2);

        // Find the ToApp message
        let app_msg = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToApp(Ok(msg)) => Some(msg.clone()),
                _ => None,
            })
            .expect("Expected ToApp message");
        assert_eq!(app_msg, message1);

        // Find the ToSlim (ack) message
        let ack_msg = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToSlim(msg) => Some(msg.clone()),
                _ => None,
            })
            .expect("Expected ToSlim ack message");
        assert_eq!(
            ack_msg.get_session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack_msg.get_session_header().get_message_id(), 1);
        assert_eq!(ack_msg.get_dst(), local_name);

        // Receive message 3 from slim direction north (skipping message 2)
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

        let output = session
            .on_message(SessionMessage::OnMessage {
                message: message3.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error receiving message3");

        // Should produce: ToSlim(ack for msg3) + ToSlim(rtx request for msg2)
        // message3 is buffered waiting for message2
        assert_eq!(output.messages.len(), 2);
        let slim_msgs: Vec<_> = output
            .messages
            .iter()
            .filter_map(|m| match m {
                OutboundMessage::ToSlim(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(slim_msgs.len(), 2);

        // Find the ack for message 3
        let ack3 = slim_msgs
            .iter()
            .find(|m| m.get_session_message_type() == ProtoSessionMessageType::MsgAck)
            .expect("Expected ack message");
        assert_eq!(ack3.get_session_header().get_message_id(), 3);

        // Find the RTX request for message 2
        let rtx_req = slim_msgs
            .iter()
            .find(|m| m.get_session_message_type() == ProtoSessionMessageType::RtxRequest)
            .expect("Expected RTX request message");
        assert_eq!(rtx_req.get_session_header().get_message_id(), 2);

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
        let output = session
            .on_message(timer_signal)
            .expect("error handling RTX timer");

        // Verify that an RTX request is produced
        assert_eq!(output.messages.len(), 1);
        let rtx_request = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m.clone(),
            _ => panic!("Expected ToSlim RTX request"),
        };
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

        let output = session
            .on_message(SessionMessage::OnMessage {
                message: message2.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error receiving message2 as RtxReply");

        // Should deliver message 2 and 3 in order to the app
        let app_messages: Vec<_> = output
            .messages
            .iter()
            .filter_map(|m| match m {
                OutboundMessage::ToApp(Ok(msg)) => Some(msg.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(app_messages.len(), 2);
        assert_eq!(app_messages[0].get_id(), 2);
        assert_eq!(app_messages[1].get_id(), 3);

        // Check that no other timeout is sent to rx_signal
        let no_timer = timeout(Duration::from_millis(300), rx_signal.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after receiving RTX reply, but got one"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end() {
        // Create two sessions, one will act as sender and one as receiver
        let session_id = 10;

        // Sender session setup
        let (tx_signal_sender, mut rx_signal_sender) = tokio::sync::mpsc::channel(10);

        let sender_name = ProtoName::from_strings(["org", "ns", "sender"]);
        let receiver_name = ProtoName::from_strings(["org", "ns", "receiver"]);

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
            tx_signal_sender.clone(),
            Direction::Bidirectional,
        );

        // Add receiver as endpoint for sender
        sender_session
            .add_endpoint(&Participant::new(
                receiver_name.clone(),
                ParticipantSettings::bidirectional(),
            ))
            .expect("error adding participant");

        // Receiver session setup
        let (tx_signal_receiver, _rx_signal_receiver) = tokio::sync::mpsc::channel(10);

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
            tx_signal_receiver.clone(),
            Direction::Bidirectional,
        );

        // Add sender as endpoint for receiver
        receiver_session
            .add_endpoint(&Participant::new(
                sender_name.clone(),
                ParticipantSettings::bidirectional(),
            ))
            .expect("error adding participant");

        // Send message 1 from the application
        let mut message1 = Message::builder()
            .source(sender_name.clone())
            .destination(receiver_name.clone())
            .application_payload("test_payload", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();
        message1.set_session_message_type(ProtoSessionMessageType::Msg);

        let output = sender_session
            .on_message(SessionMessage::OnMessage {
                message: message1.clone(),
                direction: MessageDirection::South,
                ack_tx: None,
            })
            .expect("error sending message from sender");

        // Check output from sender
        assert_eq!(output.messages.len(), 1);
        let sent_message = match &output.messages[0] {
            OutboundMessage::ToSlim(m) => m.clone(),
            _ => panic!("Expected ToSlim message"),
        };
        assert_eq!(
            sent_message.get_session_message_type(),
            ProtoSessionMessageType::Msg
        );
        assert_eq!(sent_message.get_id(), 1);
        assert_eq!(sent_message.get_dst(), receiver_name);

        // Deliver to receiver
        let mut received_message = sent_message.clone();
        received_message
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        let output = receiver_session
            .on_message(SessionMessage::OnMessage {
                message: received_message.clone(),
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error receiving message on receiver");

        // Should produce ToApp(message) + ToSlim(ack)
        let ack_message = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToSlim(msg) => Some(msg.clone()),
                _ => None,
            })
            .expect("Expected ack from receiver");
        assert_eq!(
            ack_message.get_session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(ack_message.get_session_header().get_message_id(), 1);
        assert_eq!(ack_message.get_dst(), sender_name);

        let app_message = output
            .messages
            .iter()
            .find_map(|m| match m {
                OutboundMessage::ToApp(Ok(msg)) => Some(msg.clone()),
                _ => None,
            })
            .expect("Expected app message from receiver");
        assert_eq!(app_message.get_id(), 1);
        assert_eq!(app_message.get_source(), sender_name);

        // Send ack back to sender
        let mut ack_to_sender = ack_message.clone();
        ack_to_sender
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        let output = sender_session
            .on_message(SessionMessage::OnMessage {
                message: ack_to_sender,
                direction: MessageDirection::North,
                ack_tx: None,
            })
            .expect("error processing ack on sender");
        assert!(output.messages.is_empty());

        // Wait to ensure the timer is stopped and no retransmission occurs
        let no_timer = timeout(Duration::from_millis(300), rx_signal_sender.recv()).await;
        assert!(
            no_timer.is_err(),
            "Expected no timer signal after ack, but got one"
        );
    }
}
