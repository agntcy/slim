// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

// Third-party crates
use async_trait::async_trait;
use parking_lot::RwLock;
use rand::Rng;
use slim_mls::mls::Mls;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{
    ApplicationPayload, CommandPayload, ProtoMessage as Message, ProtoSessionMessageType,
    ProtoSessionType, SessionHeader, SlimHeader,
};
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;

use crate::producer_buffer::ProducerBuffer;
use crate::receiver_buffer::ReceiverBuffer;
// Local crate
use crate::{
    Common, CommonSession, Id, MessageDirection, MessageHandler, SessionConfig, SessionConfigTrait,
    State, Transmitter,
    errors::SessionError,
    mls_state::{MlsEndpoint, MlsState},
    session_controller::{SessionController, SessionModerator, SessionParticipant},
    timer,
};

/// Configuration for the Point to Point session
#[derive(Debug, Clone, PartialEq)]
pub struct PointToPointConfiguration {
    pub timeout: Option<std::time::Duration>,
    pub max_retries: Option<u32>,
    pub mls_enabled: bool,
    pub peer_name: Option<Name>,
    pub(crate) initiator: bool,
    pub metadata: HashMap<String, String>,
}

impl Default for PointToPointConfiguration {
    fn default() -> Self {
        PointToPointConfiguration {
            timeout: None,
            max_retries: Some(5),
            mls_enabled: false,
            peer_name: None,
            initiator: true,
            metadata: HashMap::new(),
        }
    }
}

impl PointToPointConfiguration {
    pub fn new(
        timeout: Option<Duration>,
        max_retries: Option<u32>,
        mls_enabled: bool,
        peer_name: Option<Name>,
        metadata: HashMap<String, String>,
    ) -> Self {
        // If mls is enabled the session must be sticky
        if mls_enabled && peer_name.is_none() {
            panic!("MLS on not sticky sessions is not supported (must provide a peer name).");
        }

        PointToPointConfiguration {
            timeout,
            max_retries,
            mls_enabled,
            peer_name,
            initiator: true,
            metadata,
        }
    }

    pub fn with_peer_name(mut self, name: Name) -> Self {
        self.peer_name = Some(name);
        self
    }
}

impl SessionConfigTrait for PointToPointConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::PointToPoint(config) => {
                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected PointToPoint, got {:?}",
                session_config
            ))),
        }
    }
}

impl std::fmt::Display for PointToPointConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PointToPointConfiguration: timeout: {} ms, max retries: {}, remote endpoint: {}",
            self.timeout.unwrap_or_default().as_millis(),
            self.max_retries.unwrap_or_default(),
            self.peer_name
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "<unset>".to_string()),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
enum P2PSessionStatus {
    #[default]
    Uninitialized,
    Discovering,
    Established,
}

/// Message types for internal PointToPoint communication
#[allow(clippy::large_enum_variant)]
enum InternalMessage {
    OnMessage {
        message: Message,
        direction: MessageDirection,
    },
    SetConfig {
        config: PointToPointConfiguration,
    },
    TimerTimeout {
        message_id: u32,
        timeouts: u32,
        ack: bool, // true: ack timer, false: rtx timer
    },
    TimerFailure {
        message_id: u32,
        timeouts: u32,
        ack: bool, // true: ack timer, false: rtx timer
    },
}

struct SenderState {
    // buffer with packets coming from the application
    buffer: ProducerBuffer,
    // next packet id
    next_id: u32,
    // list of pending acks with timers and messages to resend
    pending_acks: HashMap<u32, (timer::Timer, Message)>,
}

struct ReceiverState {
    // buffer with received packets
    buffer: ReceiverBuffer,
    // list of pending RTX requestss
    pending_rtxs: HashMap<u32, (timer::Timer, Message)>,
}

struct PointToPointState<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    session_id: u32,
    source: Name,
    identity: String,
    tx: T,
    config: PointToPointConfiguration,
    dst: Arc<RwLock<Option<Name>>>,
    peer_connection: Option<u64>,
    p2p_session_status: P2PSessionStatus,
    send_buffer: VecDeque<Message>,
    sender_state: SenderState,     // send packets with sequential ids
    receiver_state: ReceiverState, // to be used only in case of sticky session
    channel_endpoint: Option<SessionController<P, V, T>>,
    mls: Option<Arc<Mutex<Mls<P, V>>>>,
}

impl<P, V, T> PointToPointState<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn channel_endpoint(&mut self) -> &mut SessionController<P, V, T> {
        self.channel_endpoint
            .as_mut()
            .expect("channel endpoint not initialized")
    }
}

// need two observers in order to distinguish RTX from ACK timers
struct RtxTimerObserver {
    tx: Sender<InternalMessage>,
}

struct AckTimerObserver {
    tx: Sender<InternalMessage>,
}

/// The internal part of the Point to Point session that handles message processing
struct PointToPointProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    state: PointToPointState<P, V, T>,
    ack_timer_observer: Arc<AckTimerObserver>,
    rtx_timer_observer: Arc<RtxTimerObserver>,
    rx: Receiver<InternalMessage>,
    cancellation_token: CancellationToken,
}

#[async_trait]
impl timer::TimerObserver for AckTimerObserver {
    async fn on_timeout(&self, message_id: u32, timeouts: u32) {
        let res = self
            .tx
            .send(InternalMessage::TimerTimeout {
                message_id,
                timeouts,
                ack: true,
            })
            .await;

        if let Err(e) = res {
            error!("AckTimerObserver: failed to send timer timeout: {}", e);
        }
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        // remove the state for the lost message
        let res = self
            .tx
            .send(InternalMessage::TimerFailure {
                message_id,
                timeouts,
                ack: true,
            })
            .await;

        if let Err(e) = res {
            error!("AckTimerObserver: failed to send timer failure: {}", e);
        }
    }

    async fn on_stop(&self, message_id: u32) {
        debug!("timer stopped: {}", message_id);
    }
}

#[async_trait]
impl timer::TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, message_id: u32, timeouts: u32) {
        let res = self
            .tx
            .send(InternalMessage::TimerTimeout {
                message_id,
                timeouts,
                ack: false,
            })
            .await;

        if let Err(e) = res {
            error!("RtxTimerObserver: failed to send timer timeout: {}", e);
        }
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        // remove the state for the lost message
        let res = self
            .tx
            .send(InternalMessage::TimerFailure {
                message_id,
                timeouts,
                ack: false,
            })
            .await;

        if let Err(e) = res {
            error!("RtxTimerObserver: failed to send timer failure: {}", e);
        }
    }

    async fn on_stop(&self, message_id: u32) {
        debug!("timer stopped: {}", message_id);
    }
}

impl<P, V, T> PointToPointProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn new(
        state: PointToPointState<P, V, T>,
        tx: Sender<InternalMessage>,
        rx: Receiver<InternalMessage>,
        cancellation_token: CancellationToken,
    ) -> Self {
        PointToPointProcessor {
            state,
            ack_timer_observer: Arc::new(AckTimerObserver { tx: tx.clone() }),
            rtx_timer_observer: Arc::new(RtxTimerObserver { tx: tx.clone() }),
            rx,
            cancellation_token,
        }
    }

    async fn process_loop(mut self) {
        debug!("Starting PointToPointProcessor loop");

        // Create mls state if needed
        let mls = self.state.mls.clone();
        let mls = match mls {
            Some(mls) => Some(
                MlsState::new(mls)
                    .await
                    .expect("failed to create MLS state"),
            ),
            None => None,
        };

        // Create channel endpoint to handle session discovery and encryption
        let channel_endpoint = match self.state.config.initiator {
            true => {
                let cm = SessionModerator::new(
                    self.state.source.clone(),
                    // TODO: this is set to the name of the peer if provided, otherwise to our own name
                    // This needs to be revisited, as this part should be enabled only when a peer name is provided
                    self.state
                        .config
                        .peer_name
                        .clone()
                        .unwrap_or(self.state.source.clone()),
                    self.state.session_id,
                    ProtoSessionType::PointToPoint,
                    60,
                    Duration::from_secs(1),
                    mls,
                    self.state.tx.clone(),
                    None,
                    self.state.config.metadata.clone(),
                );
                SessionController::SessionModerator(cm)
            }
            false => {
                let cp = SessionParticipant::new(
                    self.state.source.clone(),
                    // TODO: this is set to the name of the peer if provided, otherwise to our own name
                    // This needs to be revisited, as this part should be enabled only when a peer name is provided
                    self.state
                        .config
                        .peer_name
                        .clone()
                        .unwrap_or(self.state.source.clone()),
                    self.state.session_id,
                    ProtoSessionType::PointToPoint,
                    60,
                    Duration::from_secs(1),
                    mls,
                    self.state.tx.clone(),
                    self.state.config.metadata.clone(),
                );
                SessionController::SessionParticipant(cp)
            }
        };

        self.state.channel_endpoint = Some(channel_endpoint);

        // set timer for mls key rotation if it is enabled
        let sleep = time::sleep(Duration::from_secs(3600));
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                next = self.rx.recv() => {
                    match next {
                        Some(message) => self.handle_internal_message(message).await,
                        None => {
                            debug!("ff session {} channel closed", self.state.session_id);
                            break;
                        }
                    }
                }
                () = &mut sleep, if self.state.config.mls_enabled => {
                        let _ = self.state.channel_endpoint().update_mls_keys().await;
                        sleep.as_mut().reset(Instant::now() + Duration::from_secs(3600));
                }
                _ = self.cancellation_token.cancelled() => {
                    debug!("ff session {} deleted", self.state.session_id);
                    break;
                }
            }
        }

        // Finish to process any remaining messages
        while let Ok(message) = self.rx.try_recv() {
            self.handle_internal_message(message).await;
        }

        // Clean up any remaining timers
        for (_, (mut timer, _)) in self.state.sender_state.pending_acks.drain() {
            timer.stop();
        }
        for (_, (mut timer, _)) in self.state.receiver_state.pending_rtxs.drain() {
            timer.stop();
        }

        self.state.channel_endpoint().close();

        debug!("PointToPointProcessor loop exited");
    }

    async fn handle_internal_message(&mut self, message: InternalMessage) {
        match message {
            InternalMessage::OnMessage { message, direction } => {
                let result = match direction {
                    MessageDirection::North => self.handle_message_to_app(message).await,
                    MessageDirection::South => self.handle_message_to_slim(message).await,
                };

                if let Err(e) = result {
                    error!("error processing message: {}", e);
                }
            }
            InternalMessage::SetConfig { config } => {
                debug!("setting point and point session config: {}", config);
                self.state.config = config;
            }
            InternalMessage::TimerTimeout {
                message_id,
                timeouts,
                ack,
            } => {
                debug!("timer timeout for message id {}: {}", message_id, timeouts);
                self.handle_timer_timeout(message_id, ack).await;
            }
            InternalMessage::TimerFailure {
                message_id,
                timeouts,
                ack,
            } => {
                debug!("timer failure for message id {}: {}", message_id, timeouts);
                self.handle_timer_failure(message_id, ack).await;
            }
        }
    }

    async fn handle_timer_timeout(&mut self, message_id: u32, ack: bool) {
        let message = if ack {
            match self.state.sender_state.pending_acks.get(&message_id) {
                Some((_t, msg)) => msg.clone(),
                None => {
                    warn!("the timer does not exists, ignore timeout");
                    return;
                }
            }
        } else {
            match self.state.receiver_state.pending_rtxs.get(&message_id) {
                Some((_t, msg)) => msg.clone(),
                None => {
                    warn!("the timer does not exists, ignore timeout");
                    return;
                }
            }
        };

        // if RTX check if we need to send it or just remove the timer
        if !ack
            && self
                .state
                .receiver_state
                .buffer
                .message_already_received(message_id as usize)
        {
            // the message was already received, no need to send RTX
            if let Some((mut t, _m)) = self.state.receiver_state.pending_rtxs.remove(&message_id) {
                t.stop();
                return;
            }
        }

        let _ = self
            .state
            .tx
            .send_to_slim(Ok(message))
            .await
            .map_err(|e| SessionError::AppTransmission(e.to_string()));
    }

    async fn handle_timer_failure(&mut self, message_id: u32, ack: bool) {
        // Remove the state for the lost message
        let message = if ack {
            match self.state.sender_state.pending_acks.remove(&message_id) {
                Some((_timer, message)) => message,
                None => {
                    warn!(
                        "No pending ack found for message_id {} in timer failure",
                        message_id
                    );
                    return;
                }
            }
        } else {
            match self.state.receiver_state.pending_rtxs.remove(&message_id) {
                Some((_timer, message)) => message,
                None => {
                    warn!(
                        "No pending rtx found for message_id {} in timer failure",
                        message_id
                    );
                    return;
                }
            }
        };

        let _ = self
            .state
            .tx
            .send_to_app(Err(SessionError::Timeout {
                session_id: self.state.session_id,
                message_id,
                message: Box::new(message),
            }))
            .await
            .map_err(|e| SessionError::AppTransmission(e.to_string()));
    }

    async fn start_p2p_session_discovery(&mut self, name: &Name) -> Result<(), SessionError> {
        debug!("starting p2p session discovery");
        // Set payload

        let payload = Some(CommandPayload::new_discovery_request_payload(None).as_content());

        // Create a probe message
        let mut probe_message = Message::new_publish(&self.state.source, name, None, None, payload);

        let session_header = probe_message.get_session_header_mut();
        session_header.set_session_type(ProtoSessionType::PointToPoint);
        session_header.set_session_message_type(ProtoSessionMessageType::DiscoveryRequest);
        session_header.set_session_id(self.state.session_id);
        session_header.set_message_id(rand::rng().random_range(0..u32::MAX));

        self.state.p2p_session_status = P2PSessionStatus::Discovering;

        self.state
            .channel_endpoint()
            .on_message(probe_message)
            .await
    }

    async fn handle_channel_discovery_reply(
        &mut self,
        message: Message,
    ) -> Result<(), SessionError> {
        self.state.channel_endpoint().on_message(message).await
    }

    async fn handle_channel_join_request(&mut self, message: Message) -> Result<(), SessionError> {
        // Save source and incoming connection
        let source = message.get_source();
        let incoming_conn = message.get_incoming_conn();

        // pass the message to the channel endpoint
        self.state.channel_endpoint().on_message(message).await?;

        // No error - this session is now established
        *self.state.dst.write() = Some(source.clone());
        self.state.config.peer_name = Some(source);
        self.state.peer_connection = Some(incoming_conn);
        self.state.p2p_session_status = P2PSessionStatus::Established;

        Ok(())
    }

    async fn handle_channel_join_reply(&mut self, message: Message) -> Result<(), SessionError> {
        // Check if the session is established
        let source = message.get_source();
        let incoming_conn = message.get_incoming_conn();
        let status = self.state.p2p_session_status.clone();

        debug!(
            "received session discovery reply from {} and incoming conn {}",
            source,
            message.get_incoming_conn()
        );

        // send message to channel endpoint
        self.state.channel_endpoint().on_message(message).await?;

        match status {
            P2PSessionStatus::Discovering => {
                debug!("session discovery established with {}", source);

                // If we are still discovering, set the peer name
                *self.state.dst.write() = Some(source.clone());
                self.state.config.peer_name = Some(source);
                self.state.peer_connection = Some(incoming_conn);
                self.state.p2p_session_status = P2PSessionStatus::Established;

                // If MLS is not enabled, send all buffered messages
                if !self.state.config.mls_enabled {
                    // Collect messages first to avoid multiple mutable borrows
                    let messages: Vec<Message> = self.state.send_buffer.drain(..).collect();

                    // Send all buffered messages to the peer name
                    for msg in messages {
                        self.send_message(msg, None).await?;
                    }
                }

                Ok(())
            }
            _ => {
                debug!("session discovery reply received, but already established");

                // Check if the peer name is already set, and if it's different from the source
                if let Some(name) = &self.state.config.peer_name {
                    let message = if name != &source {
                        format!(
                            "session already established with a different name: {}, received: {}",
                            name, source
                        )
                    } else {
                        "session already established".to_string()
                    };

                    return Err(SessionError::AppTransmission(message));
                }

                Ok(())
            }
        }
    }

    async fn send_message(
        &mut self,
        mut message: Message,
        message_id: Option<u32>,
    ) -> Result<(), SessionError> {
        // Set the message id
        let message_id = match message_id {
            Some(id) => id,
            None => {
                let next = self.state.sender_state.next_id;
                self.state.sender_state.next_id += 1;
                next
            }
        };
        // Get a mutable reference to the message header
        let header = message.get_session_header_mut();

        // Set the session id and message id
        header.set_message_id(message_id);
        header.set_session_id(self.state.session_id);

        // If we have a peer name, set the destination to use the ID in the peer name
        // and force the message to be sent to the peer connection
        if let Some(ref name) = self.state.config.peer_name {
            let mut new_name = message.get_dst();
            new_name.set_id(name.id());
            message.get_slim_header_mut().set_destination(&new_name);

            message
                .get_slim_header_mut()
                .set_forward_to(self.state.peer_connection);
        }

        // add the message to the sender buffer
        self.state.sender_state.buffer.push(message.clone());

        if let Some(timeout_duration) = self.state.config.timeout {
            // Create timer
            let message_id = message.get_id();
            let timer = timer::Timer::new(
                message_id,
                timer::TimerType::Constant,
                timeout_duration,
                None,
                self.state.config.max_retries,
            );

            // start timer
            timer.start(self.ack_timer_observer.clone());

            // Store timer and message
            self.state
                .sender_state
                .pending_acks
                .insert(message_id, (timer, message.clone()));
        }

        // Send message
        self.state
            .tx
            .send_to_slim(Ok(message))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    pub(crate) async fn handle_message_to_slim(
        &mut self,
        mut message: Message,
    ) -> Result<(), SessionError> {
        // Set the session type
        let header = message.get_session_header_mut();
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        // If we have a peer name, decide what to do according to the session state
        if self.state.config.peer_name.is_some() {
            match self.state.p2p_session_status {
                P2PSessionStatus::Uninitialized => {
                    self.start_p2p_session_discovery(&message.get_slim_header().get_dst())
                        .await?;

                    self.state.send_buffer.push_back(message);

                    Ok(())
                }
                P2PSessionStatus::Discovering => {
                    // Still discovering the peer name. Store message in a buffer and send it later
                    // when the session is established
                    self.state.send_buffer.push_back(message);
                    Ok(())
                }
                P2PSessionStatus::Established => {
                    // the session state is established, send message
                    let mut new_name = message.get_dst();
                    new_name.set_id(self.state.config.peer_name.as_ref().unwrap().id());
                    message.get_slim_header_mut().set_destination(&new_name);
                    message
                        .get_slim_header_mut()
                        .set_forward_to(self.state.peer_connection);

                    self.send_message(message, None).await
                }
            }
        } else {
            // anycast session, just send
            self.send_message(message, None).await
        }
    }

    pub(crate) async fn handle_message_to_app(
        &mut self,
        message: Message,
    ) -> Result<(), SessionError> {
        let message_id = message.get_session_header().get_message_id();
        let source = message.get_source();
        debug!(
            %source, %message_id, "received message from slim",
        );

        // If we have a peer name, check if the source matches
        if let Some(name) = &self.state.config.peer_name
            && !(self.state.p2p_session_status == P2PSessionStatus::Discovering
                && (message.get_session_message_type() == ProtoSessionMessageType::DiscoveryReply
                    || message.get_session_message_type() == ProtoSessionMessageType::JoinReply))
            && *name != source
        {
            return Err(SessionError::AppTransmission(format!(
                "message source {} does not match peer name {}",
                source, name
            )));
        }

        match message.get_session_message_type() {
            ProtoSessionMessageType::Msg => {
                // Send an ack back as reply and forward the incoming message to the app
                // Create ack message
                let opt_ack = if self.state.config.timeout.is_some()
                    && self.state.config.max_retries.is_some()
                {
                    Some(self.create_ack(&message))
                } else {
                    None
                };

                // Forward the message to the app
                self.send_message_to_app(message).await?;

                // Send the ack if necessary
                if let Some(ack) = opt_ack {
                    self.state
                        .tx
                        .send_to_slim(Ok(ack))
                        .await
                        .map_err(|e| SessionError::SlimTransmission(e.to_string()))?
                }
                Ok(())
            }
            ProtoSessionMessageType::MsgAck => {
                // Remove the timer and drop the message
                self.stop_and_remove_timer(message_id, true)
            }
            ProtoSessionMessageType::DiscoveryReply => {
                // Handle peer session discovery
                self.handle_channel_discovery_reply(message).await
            }
            ProtoSessionMessageType::JoinRequest => {
                // Handle peer session discovery
                self.handle_channel_join_request(message).await
            }
            ProtoSessionMessageType::JoinReply => {
                // Handle peer session discovery reply
                self.handle_channel_join_reply(message).await
            }
            ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck => {
                // Handle mls stuff
                self.state.channel_endpoint().on_message(message).await?;

                // Flush the send buffer if MLS is enabled
                if self.state.channel_endpoint().is_mls_up()? {
                    // If MLS is up, send all buffered messages
                    let messages: Vec<Message> = self.state.send_buffer.drain(..).collect();

                    for msg in messages {
                        self.send_message(msg, None).await?;
                    }
                }

                Ok(())
            }
            ProtoSessionMessageType::RtxRequest => {
                // Received an RTX request, try to reply
                self.process_incoming_rtx_request(message).await
            }
            ProtoSessionMessageType::RtxReply => {
                // Received an RTX reply, process it
                self.process_incoming_rtx_reply(message).await
            }
            _ => {
                // Unexpected header
                Err(SessionError::AppTransmission(format!(
                    "invalid session header {}",
                    message.get_session_message_type() as u32
                )))
            }
        }
    }

    async fn process_incoming_rtx_request(&mut self, message: Message) -> Result<(), SessionError> {
        let msg_rtx_id = message.get_id();
        let pkt_src = message.get_source();
        let pkt_dst = message.get_dst();
        let incoming_conn = message.get_incoming_conn();
        let session_id = message.get_session_header().session_id;

        let rtx_pub = match self.state.sender_state.buffer.get(msg_rtx_id as usize) {
            Some(packet) => {
                debug!(
                    "packet {} exists in the producer buffer, create rtx reply",
                    msg_rtx_id
                );

                // the packet exists, send it to the source of the RTX
                let payload = match packet.get_payload() {
                    Some(p) => p,
                    None => {
                        error!("unable to get the payload from the packet, do not send packet");
                        return Err(SessionError::MessageLost(msg_rtx_id.to_string()));
                    }
                };

                let flags = SlimHeaderFlags::default()
                    .with_forward_to(incoming_conn)
                    .with_fanout(1);

                let slim_header = Some(SlimHeader::new(
                    &pkt_dst,
                    &pkt_src,
                    &self.state.identity,
                    Some(flags),
                ));

                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::PointToPoint.into(),
                    ProtoSessionMessageType::RtxReply.into(),
                    session_id,
                    msg_rtx_id,
                ));

                Message::new_publish_with_headers(
                    slim_header,
                    session_header,
                    Some(payload.clone()),
                )
            }
            None => {
                // the packet does not exist return an empty RtxReply with the error flag set
                debug!(
                    "received an RTX messages for an old packet on session {}",
                    session_id
                );

                let flags = SlimHeaderFlags::default()
                    .with_forward_to(incoming_conn)
                    .with_error(true);

                let slim_header = Some(SlimHeader::new(
                    &pkt_dst,
                    &pkt_src,
                    &self.state.identity,
                    Some(flags),
                ));

                // no need to set source and destiona here
                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::PointToPoint.into(),
                    ProtoSessionMessageType::RtxReply.into(),
                    session_id,
                    msg_rtx_id,
                ));

                let payload = Some(ApplicationPayload::new("", vec![]).as_content());

                Message::new_publish_with_headers(slim_header, session_header, payload)
            }
        };

        self.state.tx.send_to_slim(Ok(rtx_pub)).await
    }

    async fn process_incoming_rtx_reply(&mut self, message: Message) -> Result<(), SessionError> {
        // Remove RTX timer
        let msg_id = message.get_session_header().get_message_id();
        self.stop_and_remove_timer(msg_id, false)?;

        let ack = self.create_ack(&message);

        // Forward the message to the app
        self.send_message_to_app(message).await?;

        // Send the ack
        self.state
            .tx
            .send_to_slim(Ok(ack))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    fn create_ack(&self, message: &Message) -> Message {
        let src = message.get_source();
        let message_id = message.get_session_header().message_id;
        let slim_header = Some(SlimHeader::new(
            &self.state.source,
            &src,
            &self.state.identity,
            Some(SlimHeaderFlags::default().with_forward_to(message.get_incoming_conn())),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::MsgAck.into(),
            message.get_session_header().session_id,
            message_id,
        ));

        let payload = Some(ApplicationPayload::new("", vec![]).as_content());

        Message::new_publish_with_headers(slim_header, session_header, payload)
    }

    /// Helper function to send a message to the application.
    /// This is used by both the P2p and F2pReliable message handlers.
    async fn send_message_to_app(&mut self, message: Message) -> Result<(), SessionError> {
        // if the session is not reliable or we don't have a peer we can accept holes in the
        // sequence of the received messages and so we send this packets to the application
        // immediately without reordering. notice that an anycast reliable session is possible
        // and the packet are re-sent by the sender if acks are not received
        if message.get_session_message_type() == ProtoSessionMessageType::Msg
            || (!self.state.config.mls_enabled && self.state.config.peer_name.is_none())
        {
            // this is an anycast session so simply send the message to the app
            return self
                .state
                .tx
                .send_to_app(Ok(message))
                .await
                .map_err(|e| SessionError::SlimTransmission(e.to_string()));
        }

        // here we need to reorder the messages if needed
        let session_id = message.get_session_header().session_id;

        let recv;
        let mut rtx = Vec::new();
        if message.get_session_message_type() == ProtoSessionMessageType::RtxReply
            && message.get_error().is_some()
            && message.get_error().unwrap()
        {
            // this is a packet that cannot be recovered anymore
            recv = self
                .state
                .receiver_state
                .buffer
                .on_lost_message(message.get_session_header().get_message_id());
        } else {
            let (r, rtx_vec) = self
                .state
                .receiver_state
                .buffer
                .on_received_message(message);
            recv = r;
            rtx = rtx_vec;
        }

        for opt in recv {
            match opt {
                Some(m) => {
                    // send message to the app
                    if self.state.tx.send_to_app(Ok(m)).await.is_err() {
                        error!("error sending packet to app on session {}", session_id);
                    };
                }
                None => {
                    warn!("a message was definitely lost in session {}", session_id);
                    if self
                        .state
                        .tx
                        .send_to_app(Err(SessionError::MessageLost(session_id.to_string())))
                        .await
                        .is_err()
                    {
                        error!("error notifiyng missing packet to session {}", session_id);
                    };
                }
            }
        }

        // if rtx is not empty we detected at least one missing packet.
        // the packet may be in flight (the sender keeps sending packet if no ack is received,
        // however the max retransmission number can be hit). So avoid, to get stuck and do not
        // send any packet to the application the receiver can also ask for retransmissions.
        // doing so if a packet is available in the sender buffer it will be receoverd.

        let destination = match &self.state.config.peer_name {
            Some(d) => d,
            None => {
                warn!("cannot send rtx messages, destination name is missing");
                return Err(SessionError::MessageLost(session_id.to_string()));
            }
        };

        let connection = match self.state.peer_connection {
            Some(c) => c,
            None => {
                warn!("cannot send rtx messages, incoming connection is missing");
                return Err(SessionError::MessageLost(session_id.to_string()));
            }
        };

        if !rtx.is_empty() {
            for msg_id in rtx {
                let timer_duration = self.state.config.timeout.unwrap_or(Duration::from_secs(1));
                let timer = timer::Timer::new(
                    msg_id,
                    timer::TimerType::Constant,
                    timer_duration,
                    None,
                    self.state.config.max_retries,
                );

                // create RTX packet
                let slim_header = Some(SlimHeader::new(
                    &self.state.source,
                    destination,
                    &self.state.identity,
                    Some(SlimHeaderFlags::default().with_forward_to(connection)),
                ));

                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::PointToPoint.into(),
                    ProtoSessionMessageType::RtxRequest.into(),
                    self.state.session_id,
                    msg_id,
                ));

                let payload = Some(ApplicationPayload::new("", vec![]).as_content());
                let rtx = Message::new_publish_with_headers(slim_header, session_header, payload);

                // start timer
                timer.start(self.rtx_timer_observer.clone());

                // Store timer and message
                self.state
                    .receiver_state
                    .pending_rtxs
                    .insert(msg_id, (timer, rtx));
            }
        }

        Ok(())
    }

    /// Helper function to stop and remove a timer by message ID.
    /// Returns Ok(()) if the timer was found and stopped, or an appropriate error if not.
    fn stop_and_remove_timer(&mut self, message_id: u32, ack: bool) -> Result<(), SessionError> {
        if ack {
            match self.state.sender_state.pending_acks.remove(&message_id) {
                Some((mut timer, _message)) => {
                    // Stop the timer
                    timer.stop();
                    Ok(())
                }
                None => Err(SessionError::AppTransmission(format!(
                    "timer not found for message id {}",
                    message_id
                ))),
            }
        } else {
            match self.state.receiver_state.pending_rtxs.remove(&message_id) {
                Some((mut timer, _message)) => {
                    // Stop the timer
                    timer.stop();
                    Ok(())
                }
                None => Err(SessionError::AppTransmission(format!(
                    "timer not found for message id {}",
                    message_id
                ))),
            }
        }
    }
}

/// The interface for the point to point session
#[derive(Debug)]
pub(crate) struct PointToPoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    common: Common<P, V, T>,
    tx: Sender<InternalMessage>,
    cancellation_token: CancellationToken,
}
impl<P, V, T> PointToPoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: PointToPointConfiguration,
        name: Name,
        tx_slim_app: T,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);

        let identity = identity_provider
            .get_token()
            .expect("failed to get the local identity");

        // Common session stuff
        let common = Common::new(
            id,
            SessionConfig::PointToPoint(session_config.clone()),
            name,
            tx_slim_app.clone(),
            identity_provider,
            identity_verifier,
            session_config.mls_enabled,
            storage_path,
        );

        if let Some(remote) = session_config.peer_name.clone() {
            common.set_dst(remote);
        }

        // PointToPoint internal state
        let state = PointToPointState {
            session_id: id,
            source: common.source().clone(),
            identity,
            tx: tx_slim_app.clone(),
            config: session_config,
            dst: common.dst_arc(),
            peer_connection: None,
            p2p_session_status: P2PSessionStatus::Uninitialized,
            send_buffer: VecDeque::new(),
            sender_state: SenderState {
                buffer: ProducerBuffer::with_capacity(500),
                next_id: 0,
                pending_acks: HashMap::new(),
            },
            receiver_state: ReceiverState {
                buffer: ReceiverBuffer::default(),
                pending_rtxs: HashMap::new(),
            },
            channel_endpoint: None,
            mls: common.mls(),
        };

        // Cancellation token
        let cancellation_token = CancellationToken::new();

        // Create the processor
        let processor =
            PointToPointProcessor::new(state, tx.clone(), rx, cancellation_token.clone());

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        PointToPoint {
            common,
            tx,
            cancellation_token,
        }
    }

    pub fn with_dst<R>(&self, f: impl FnOnce(Option<&Name>) -> R) -> R {
        self.common.with_dst(f)
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for PointToPoint<P, V, T>
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
        self.common.set_session_config(session_config)?;

        // Also set the config in the processor
        let tx = self.tx.clone();
        let config = match session_config {
            SessionConfig::PointToPoint(config) => config.clone(),
            _ => {
                return Err(SessionError::ConfigurationError(
                    "invalid session config type".to_string(),
                ));
            }
        };

        tokio::spawn(async move {
            let res = tx.send(InternalMessage::SetConfig { config }).await;
            if let Err(e) = res {
                error!("failed to send config update: {}", e);
            }
        });

        Ok(())
    }

    fn source(&self) -> &Name {
        self.common.source()
    }

    fn dst(&self) -> Option<Name> {
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
        self.common.set_dst(dst)
    }
}

impl<P, V, T> Drop for PointToPoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        // Signal the processor to stop
        self.cancellation_token.cancel();
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for PointToPoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx
            .try_send(InternalMessage::OnMessage { message, direction })
            .map_err(|e| SessionError::SessionClosed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::RwLock;
    use slim_auth::shared_secret::SharedSecret;
    use std::time::Duration;
    use tracing_test::traced_test;

    use super::*;
    use crate::{
        session_controller::handle_channel_discovery_message, transmitter::SessionTransmitter,
    };
    use slim_datapath::{api::ProtoMessage, messages::Name};
    use slim_testing::utils::TEST_VALID_SECRET;

    #[tokio::test]
    async fn test_point_to_point_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let session = PointToPoint::new(
            0,
            PointToPointConfiguration::default(),
            source.clone(),
            tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::PointToPoint(PointToPointConfiguration::default())
        );
    }

    #[tokio::test]
    async fn test_point_to_point_create_with_remote_dst() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);
        let remote = Name::from_strings(["cisco", "default", "remote"]).with_id(999);

        let config = PointToPointConfiguration::default().with_peer_name(remote.clone());

        let session = PointToPoint::new(
            0,
            config,
            source.clone(),
            tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        assert_eq!(session.dst(), Some(remote));
    }

    #[tokio::test]
    async fn test_point_to_point_on_message() {
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        // SessionTransmitter::new expects (slim_tx, app_tx)
        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let session = PointToPoint::new(
            0,
            PointToPointConfiguration::default(),
            source.clone(),
            tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let payload = Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content());

        let mut message = ProtoMessage::new_publish(
            &source,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            None,
            payload,
        );

        // set the session id in the message (session created with id 0)
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_message_id(), 0);
    }

    #[tokio::test]
    async fn test_point_to_point_on_message_with_ack() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let session = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled: false,
                peer_name: None,
                initiator: true,
                metadata: HashMap::new(),
            },
            source.clone(),
            tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let payload = Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content());

        let mut message = ProtoMessage::new_publish(
            &source,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            payload,
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.message_id = 12345;
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_message_id(), 12345);
        assert_eq!(msg.get_session_header().get_session_id(), 0);

        let msg = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = msg.get_session_header();
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::MsgAck
        );
        assert_eq!(header.get_message_id(), 12345);
    }

    #[tokio::test]
    async fn test_point_to_point_timers_until_error() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        // SessionTransmitter::new expects (slim_tx, app_tx)
        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let session = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled: false,
                peer_name: None,
                initiator: true,
                metadata: HashMap::new(),
            },
            source.clone(),
            tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let mut message = ProtoMessage::new_publish(
            &source,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            None,
            Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content()),
        );

        let res = session
            .on_message(message.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        // set the session id in the message for the comparison inside the for loop
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.set_session_message_type(ProtoSessionMessageType::Msg);
        header.set_session_type(ProtoSessionType::PointToPoint);

        for _i in 0..6 {
            let mut msg = rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            // msg must be the same as message, except for the random message_id
            let header = msg.get_session_header_mut();
            header.message_id = 0;
            assert_eq!(msg, message);
        }

        let msg = rx_app.recv().await.expect("no message received");
        assert!(msg.is_err());
    }

    #[tokio::test]
    async fn test_point_to_point_timers_and_ack() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::unbounded_channel();

        let tx_sender = SessionTransmitter::new(tx_slim_sender, tx_app_sender);

        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::unbounded_channel();

        let tx_receiver = SessionTransmitter::new(tx_slim_receiver, tx_app_receiver);

        let local = Name::from_strings(["cisco", "default", "local"]).with_id(0);
        let remote = Name::from_strings(["cisco", "default", "remote"]).with_id(0);

        let session_sender = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled: false,
                peer_name: None,
                initiator: true,
                metadata: HashMap::new(),
            },
            local.clone(),
            tx_sender,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        // this can be a standard p2p session
        let session_recv = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled: false,
                peer_name: None,
                initiator: false,
                metadata: HashMap::new(),
            },
            remote.clone(),
            tx_receiver,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let mut message = ProtoMessage::new_publish(
            &local,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content()),
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.set_session_id(0);
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        let res = session_sender
            .on_message(message.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        // get one message and drop it to kick in the timers
        let mut msg = rx_slim_sender
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        // msg must be the same as message, except for the rundom message_id
        let header = msg.get_session_header_mut();
        header.set_message_id(0);
        assert_eq!(msg, message);

        // this is the first RTX
        let msg = rx_slim_sender
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // this second message is received by the receiver
        let res = session_recv
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the message should be delivered to the app
        let mut msg = rx_app_receiver
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        // msg must be the same as message, except for the random message_id
        let header = msg.get_session_header_mut();
        header.set_message_id(0);
        assert_eq!(msg, message);

        // the session layer should generate an ack
        let ack = rx_slim_receiver
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = ack.get_session_header();
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::MsgAck
        );

        // Check that the ack is sent back to the sender
        assert_eq!(message.get_source(), ack.get_dst());

        // deliver the ack to the sender
        let res = session_sender
            .on_message(ack.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter::new(tx_slim, tx_app);

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        {
            let _session = PointToPoint::new(
                0,
                PointToPointConfiguration::default(),
                source.clone(),
                tx,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
                std::path::PathBuf::from("/tmp/test_session"),
            );
        }

        // sleep for a bit to let the session drop
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }

    async fn template_test_point_to_point_session(mls_enabled: bool) {
        let (sender_tx_slim, mut sender_rx_slim) = tokio::sync::mpsc::channel(16);
        let (sender_tx_app, _sender_rx_app) = tokio::sync::mpsc::unbounded_channel();

        let sender_tx = SessionTransmitter {
            slim_tx: sender_tx_slim,
            app_tx: sender_tx_app,
            interceptors: Arc::new(RwLock::new(Vec::new())),
        };

        let (receiver_tx_slim, mut receiver_rx_slim) = tokio::sync::mpsc::channel(16);
        let (receiver_tx_app, mut receiver_rx_app) = tokio::sync::mpsc::unbounded_channel();

        let receiver_tx = SessionTransmitter {
            slim_tx: receiver_tx_slim,
            app_tx: receiver_tx_app,
            interceptors: Arc::new(RwLock::new(Vec::new())),
        };

        let local = Name::from_strings(["cisco", "default", "local"]).with_id(0);
        let remote = Name::from_strings(["cisco", "default", "remote"]).with_id(0);

        let sender_session = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled,
                peer_name: Some(remote.clone()),
                initiator: true,
                metadata: HashMap::new(),
            },
            local.clone(),
            sender_tx,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_sender"),
        );

        let receiver_session = PointToPoint::new(
            0,
            PointToPointConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                mls_enabled,
                peer_name: None,
                initiator: false,
                metadata: HashMap::new(),
            },
            remote.clone(),
            receiver_tx,
            SharedSecret::new("b", TEST_VALID_SECRET),
            SharedSecret::new("b", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_receiver"),
        );

        // Create a message to send
        let mut message = ProtoMessage::new_publish(
            &local,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            None,
            Some(ApplicationPayload::new("", vec![0x1, 0x2, 0x3, 0x4]).as_content()),
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.set_session_id(0);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        // set a fake incoming connection id
        let slim_header = message.get_slim_header_mut();
        slim_header.set_incoming_conn(Some(0));

        // Send the message
        sender_session
            .on_message(message.clone(), MessageDirection::South)
            .await
            .expect("failed to send message");

        // We should now get a session discovery message
        let mut msg = sender_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // Set fake incoming connection id
        msg.set_incoming_conn(Some(0));

        let header = msg.get_session_header_mut();
        header.set_session_message_type(ProtoSessionMessageType::DiscoveryRequest);

        // assert something
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::DiscoveryRequest,
        );

        assert_eq!(msg.get_session_type(), ProtoSessionType::PointToPoint);

        // create a discovery reply message. this is normally originated by the session layer
        let mut discovery_reply = handle_channel_discovery_message(
            &msg,
            &remote,
            receiver_session.id(),
            ProtoSessionType::PointToPoint,
        );
        discovery_reply.set_incoming_conn(Some(0));

        // Pass discovery reply message to the sender session
        sender_session
            .on_message(discovery_reply, MessageDirection::North)
            .await
            .expect("failed to handle discovery reply");

        // Sender should now issue a subscribe and a set route message - ignore them
        let _ = sender_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let _ = sender_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // Sender should then issue a channel join request message
        let mut msg = sender_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = msg.get_session_header();

        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::JoinRequest
        );

        assert_eq!(header.session_type(), ProtoSessionType::PointToPoint);

        // Set a fake incoming connection id
        msg.set_incoming_conn(Some(0));

        // Pass the channel join request message to the receiver session
        receiver_session
            .on_message(msg.clone(), MessageDirection::North)
            .await
            .expect("failed to handle channel join request");

        // We should get first the set route message
        let _ = receiver_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // And then the channel join reply message
        let mut msg = receiver_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = msg.get_session_header();

        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::JoinReply
        );

        assert_eq!(header.session_type(), ProtoSessionType::PointToPoint);

        // Pass the channel join reply message to the sender session
        msg.set_incoming_conn(Some(0));
        sender_session
            .on_message(msg, MessageDirection::North)
            .await
            .expect("failed to handle channel join reply");

        // wait one moment
        tokio::time::sleep(Duration::from_millis(100)).await;

        // After channel join reply only the sender (initiator, sticky) should have dst set
        assert_eq!(sender_session.dst(), Some(remote.clone()));
        assert_eq!(receiver_session.dst(), Some(local.clone()));

        // Check the payload
        if mls_enabled {
            // If MLS is enabled, the sender session should now send an MlsWelcome message
            let mut msg = sender_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            let header = msg.get_session_header();

            assert_eq!(
                header.session_message_type(),
                ProtoSessionMessageType::GroupWelcome
            );

            assert_eq!(header.session_type(), ProtoSessionType::PointToPoint);

            // Set a fake incoming connection id
            msg.set_incoming_conn(Some(0));

            // Pass the MlsWelcome message to the receiver session
            receiver_session
                .on_message(msg, MessageDirection::North)
                .await
                .expect("failed to handle mls welcome");

            // We should now get an ack message back
            let mut msg = receiver_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            let header = msg.get_session_header();
            assert_eq!(
                header.session_message_type(),
                ProtoSessionMessageType::GroupAck
            );

            assert_eq!(header.session_type(), ProtoSessionType::PointToPoint);

            // Send the ack to the sender session
            msg.set_incoming_conn(Some(0));
            sender_session
                .on_message(msg, MessageDirection::North)
                .await
                .expect("failed to handle mls ack");

            // Now we should get the original message
            let mut msg = sender_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            let header = msg.get_session_header();

            assert_eq!(header.session_message_type(), ProtoSessionMessageType::Msg);

            assert_eq!(header.session_type(), ProtoSessionType::PointToPoint);

            // As MLS is enabled, the payload should be encrypted
            tracing::info!(
                "Checking if payload is encrypted {}",
                msg.get_payload()
                    .unwrap()
                    .as_application_payload()
                    .blob
                    .len()
            );
            assert!(
                !msg.get_payload()
                    .unwrap()
                    .as_application_payload()
                    .blob
                    .is_empty()
            );
            assert_ne!(msg.get_payload(), message.get_payload());

            // Pass message to the receiver session
            msg.set_incoming_conn(Some(0));
            receiver_session
                .on_message(msg, MessageDirection::North)
                .await
                .expect("failed to handle message");

            // Get message from the receiver app
            let msg = receiver_rx_app
                .recv()
                .await
                .expect("no message received")
                .expect("error");
            assert_eq!(msg.get_payload(), message.get_payload());
        } else {
            // The sender session should now send the original message to the receiver
            let mut msg = sender_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");
            let header = msg.get_session_header();
            assert_eq!(header.session_message_type(), ProtoSessionMessageType::Msg);

            msg.set_incoming_conn(Some(0));

            assert_eq!(msg.get_payload(), message.get_payload());
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_point_to_point_session_no_mls() {
        template_test_point_to_point_session(false).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_point_to_point_session_mls() {
        template_test_point_to_point_session(true).await;
    }
}
