// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoSessionType, SessionHeader, SlimHeader};
use slim_datapath::messages::AgentType;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::channel_endpoint::{
    ChannelEndpoint, ChannelModerator, ChannelParticipant, MlsEndpoint, MlsState,
};
use crate::errors::SessionError;
use crate::session::{
    Common, CommonSession, Id, MessageDirection, MessageHandler, SessionConfig, SessionConfigTrait,
    SessionDirection, SessionMessage, SessionTransmitter, State,
};
use crate::timer;
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType};
use slim_datapath::messages::encoder::Agent;
use slim_datapath::messages::utils::SlimHeaderFlags;

/// Configuration for the Fire and Forget session
#[derive(Debug, Clone, PartialEq)]
pub struct FireAndForgetConfiguration {
    pub timeout: Option<std::time::Duration>,
    pub max_retries: Option<u32>,
    pub sticky: bool,
    pub mls_enabled: bool,
    pub(crate) initiator: bool,
}

impl Default for FireAndForgetConfiguration {
    fn default() -> Self {
        FireAndForgetConfiguration {
            timeout: None,
            max_retries: Some(5),
            sticky: false,
            mls_enabled: false,
            initiator: true,
        }
    }
}

impl FireAndForgetConfiguration {
    pub fn new(
        timeout: Option<Duration>,
        max_retries: Option<u32>,
        mut sticky: bool,
        mls_enabled: bool,
    ) -> Self {
        // If mls is enabled and session is not sticky, print a warning
        if mls_enabled && !sticky {
            warn!("MLS on non-sticky sessions is not supported yet. Forcing sticky session.");

            sticky = true;
        }

        FireAndForgetConfiguration {
            timeout,
            max_retries,
            sticky,
            mls_enabled,
            initiator: true,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Default)]
enum StickySessionStatus {
    #[default]
    Uninitialized,
    Discovering,
    Established,
}

/// Message types for internal FireAndForget communication
#[allow(clippy::large_enum_variant)]
enum InternalMessage {
    OnMessage {
        message: SessionMessage,
        direction: MessageDirection,
    },
    SetConfig {
        config: FireAndForgetConfiguration,
    },
    TimerTimeout {
        message_id: u32,
        timeouts: u32,
    },
    TimerFailure {
        message_id: u32,
        timeouts: u32,
    },
}

struct FireAndForgetState<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    session_id: u32,
    source: Agent,
    tx: T,
    config: FireAndForgetConfiguration,
    timers: HashMap<u32, (timer::Timer, Message)>,
    sticky_name: Option<Agent>,
    sticky_connection: Option<u64>,
    sticky_session_status: StickySessionStatus,
    sticky_buffer: VecDeque<Message>,
    channel_endpoint: ChannelEndpoint<P, V, T>,
}

struct RtxTimerObserver {
    tx: Sender<InternalMessage>,
}

/// The internal part of the Fire and Forget session that handles message processing
struct FireAndForgetProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    state: FireAndForgetState<P, V, T>,
    timer_observer: Arc<RtxTimerObserver>,
    rx: Receiver<InternalMessage>,
    cancellation_token: CancellationToken,
}

#[async_trait]
impl timer::TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, message_id: u32, timeouts: u32) {
        self.tx
            .send(InternalMessage::TimerTimeout {
                message_id,
                timeouts,
            })
            .await
            .expect("failed to send timer timeout");
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        // remove the state for the lost message
        self.tx
            .send(InternalMessage::TimerFailure {
                message_id,
                timeouts,
            })
            .await
            .expect("failed to send timer failure");
    }

    async fn on_stop(&self, message_id: u32) {
        debug!("timer stopped: {}", message_id);
    }
}

impl<P, V, T> FireAndForgetProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    fn new(
        state: FireAndForgetState<P, V, T>,
        tx: Sender<InternalMessage>,
        rx: Receiver<InternalMessage>,
        cancellation_token: CancellationToken,
    ) -> Self {
        FireAndForgetProcessor {
            state,
            timer_observer: Arc::new(RtxTimerObserver { tx: tx.clone() }),
            rx,
            cancellation_token,
        }
    }

    async fn process_loop(mut self) {
        debug!("Starting FireAndForgetProcessor loop");

        // set timer for mls key rotation if it is enabled
        let sleep = time::sleep(Duration::from_secs(3600));
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                next = self.rx.recv() => {
                    match next {
                        Some(message) => match message {
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
                                debug!("setting fire and forget session config: {}", config);
                                self.state.config = config;
                            }
                            InternalMessage::TimerTimeout {
                                message_id,
                                timeouts,
                            } => {
                                debug!("timer timeout for message id {}: {}", message_id, timeouts);
                                self.handle_timer_timeout(message_id).await;
                            }
                            InternalMessage::TimerFailure {
                                message_id,
                                timeouts,
                            } => {
                                debug!("timer failure for message id {}: {}", message_id, timeouts);
                                self.handle_timer_failure(message_id).await;
                            }
                        },
                        None => {
                            debug!("ff session {} channel closed", self.state.session_id);
                            break;
                        }
                    }
                }
                () = &mut sleep, if self.state.config.mls_enabled => {
                        let _ = self.state.channel_endpoint.update_mls_keys().await;
                        sleep.as_mut().reset(Instant::now() + Duration::from_secs(3600));
                }
                _ = self.cancellation_token.cancelled() => {
                    debug!("ff session {} deleted", self.state.session_id);
                    break;
                }
            }
        }

        // Clean up any remaining timers
        for (_, (mut timer, _)) in self.state.timers.drain() {
            timer.stop();
        }

        debug!("FireAndForgetProcessor loop exited");
    }

    async fn handle_timer_timeout(&mut self, message_id: u32) {
        // Try to send the message again
        if let Some((_timer, message)) = self.state.timers.get(&message_id) {
            let msg = message.clone();

            let _ = self
                .state
                .tx
                .send_to_slim(Ok(msg))
                .await
                .map_err(|e| SessionError::AppTransmission(e.to_string()));
        }
    }

    async fn handle_timer_failure(&mut self, message_id: u32) {
        // Remove the state for the lost message
        if let Some((_timer, message)) = self.state.timers.remove(&message_id) {
            let _ = self
                .state
                .tx
                .send_to_app(Err(SessionError::Timeout {
                    session_id: self.state.session_id,
                    message_id,
                    message: Box::new(SessionMessage::from(message)),
                }))
                .await
                .map_err(|e| SessionError::AppTransmission(e.to_string()));
        }
    }

    async fn start_sticky_session_discovery(
        &mut self,
        agent_type: &AgentType,
    ) -> Result<(), SessionError> {
        debug!("starting sticky session discovery");

        // Set payload
        let payload = bincode::encode_to_vec(&self.state.source, bincode::config::standard())
            .map_err(|e| SessionError::Processing(e.to_string()))?;

        // Create a probe message to discover the sticky session
        let mut probe_message = Message::new_publish(
            &self.state.source,
            agent_type,
            None,
            None,
            "sticky_session_discovery",
            payload,
        );

        let session_header = probe_message.get_session_header_mut();
        session_header.set_session_type(ProtoSessionType::SessionFireForget);
        session_header.set_session_message_type(ProtoSessionMessageType::ChannelDiscoveryRequest);
        session_header.set_session_id(self.state.session_id);
        session_header.set_message_id(rand::rng().random_range(0..u32::MAX));

        self.state.sticky_session_status = StickySessionStatus::Discovering;

        self.state.channel_endpoint.on_message(probe_message).await
    }

    async fn handle_channel_discovery_reply(
        &mut self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        self.state
            .channel_endpoint
            .on_message(message.message)
            .await
    }

    async fn handle_channel_join_request(
        &mut self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        // Save source and incoming connection
        let source = message.message.get_source();
        let incoming_conn = message.message.get_incoming_conn();

        // pass the message to the channel endpoint
        self.state
            .channel_endpoint
            .on_message(message.message)
            .await?;

        // No error - this session is sticky
        self.state.sticky_name = Some(source);
        self.state.sticky_connection = Some(incoming_conn);
        self.state.sticky_session_status = StickySessionStatus::Established;

        Ok(())
    }

    async fn handle_channel_join_reply(
        &mut self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        // Check if the sticky session is established
        let source = message.message.get_source();
        let incoming_conn = message.message.get_incoming_conn();
        let status = self.state.sticky_session_status.clone();

        debug!(
            "received sticky session discovery reply from {} and incoming conn {}",
            source,
            message.message.get_incoming_conn()
        );

        // send message to channel endpoint
        self.state
            .channel_endpoint
            .on_message(message.message)
            .await?;

        match status {
            StickySessionStatus::Discovering => {
                debug!("sticky session discovery established with {}", source);

                // If we are still discovering, set the sticky name
                self.state.sticky_name = Some(source);
                self.state.sticky_connection = Some(incoming_conn);
                self.state.sticky_session_status = StickySessionStatus::Established;

                // If MLS is not enabled, send all buffered messages
                if !self.state.config.mls_enabled {
                    // Collect messages first to avoid multiple mutable borrows
                    let messages: Vec<Message> = self.state.sticky_buffer.drain(..).collect();

                    // Send all buffered messages to the sticky name
                    for msg in messages {
                        self.send_message(msg, None).await?;
                    }
                }

                Ok(())
            }
            _ => {
                debug!("sticky session discovery reply received, but already established");

                // Check if the sticky name is already set, and if it's different from the source
                if let Some(name) = &self.state.sticky_name {
                    let message = if name != &source {
                        format!(
                            "sticky session already established with a different name: {}, received: {}",
                            name, source
                        )
                    } else {
                        "sticky session already established".to_string()
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
        // Set the message id to a random value
        let message_id = message_id.unwrap_or_else(|| rand::rng().random_range(0..u32::MAX));

        // Get a mutable reference to the message header
        let header = message.get_session_header_mut();

        // Set the session id and message id
        header.set_message_id(message_id);
        header.set_session_id(self.state.session_id);

        // If we have a sticky name, set the destination to the sticky name
        // and force the message to be sent to the sticky connection
        if let Some(ref name) = self.state.sticky_name {
            message.get_slim_header_mut().set_destination(name);
            message
                .get_slim_header_mut()
                .set_forward_to(self.state.sticky_connection);
        }

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
            timer.start(self.timer_observer.clone());

            // Store timer and message
            self.state
                .timers
                .insert(message_id, (timer, message.clone()));
        }

        debug!(
            "sending sticky session discovery reply to {}",
            message.get_source()
        );

        // Send message
        self.state
            .tx
            .send_to_slim(Ok(message))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    pub(crate) async fn handle_message_to_slim(
        &mut self,
        mut message: SessionMessage,
    ) -> Result<(), SessionError> {
        // Reference to session info
        let info = &message.info;

        // Set the session type
        let header = message.message.get_session_header_mut();
        header.set_session_type(if info.session_type_unset() {
            ProtoSessionType::SessionFireForget
        } else {
            info.get_session_type()
        });
        if self.state.config.timeout.is_some() {
            header.set_session_message_type(if info.session_message_type_unset() {
                ProtoSessionMessageType::FnfReliable
            } else {
                info.get_session_message_type()
            });
        } else {
            header.set_session_message_type(if info.session_message_type_unset() {
                ProtoSessionMessageType::FnfMsg
            } else {
                info.get_session_message_type()
            });
        }

        // If session is sticky, and we have a sticky name, set the destination
        // to the sticky name
        if self.state.config.sticky {
            match self.state.sticky_name {
                Some(ref name) => {
                    message.message.get_slim_header_mut().set_destination(name);
                    message
                        .message
                        .get_slim_header_mut()
                        .set_forward_to(self.state.sticky_connection);
                }
                None => {
                    let ret = match self.state.sticky_session_status {
                        StickySessionStatus::Uninitialized => {
                            self.start_sticky_session_discovery(
                                &message.message.get_slim_header().get_dst().0,
                            )
                            .await?;

                            self.state.sticky_buffer.push_back(message.message);

                            Ok(())
                        }
                        StickySessionStatus::Established => {
                            // This should not happen, as we should have a sticky name
                            Err(SessionError::AppTransmission(
                                "sticky session already established".to_string(),
                            ))
                        }
                        StickySessionStatus::Discovering => {
                            // Still discovering the sticky session. Store message in a buffer and send it later
                            // when the sticky session is established
                            self.state.sticky_buffer.push_back(message.message);
                            Ok(())
                        }
                    };

                    return ret;
                }
            }
        }

        self.send_message(message.message, message.info.message_id)
            .await
    }

    pub(crate) async fn handle_message_to_app(
        &mut self,
        message: SessionMessage,
    ) -> Result<(), SessionError> {
        let message_id = message.info.message_id.expect("message id not found");
        let source = message.message.get_source();
        debug!(
            %source, %message_id, "received message from slim",
        );

        // If session is sticky, check if the source matches the sticky name
        if self.state.config.sticky {
            if let Some(name) = &self.state.sticky_name {
                let source = message.message.get_source();
                if *name != source {
                    return Err(SessionError::AppTransmission(format!(
                        "message source {} does not match sticky name {}",
                        source, name
                    )));
                }
            }
        }

        match message.message.get_session_message_type() {
            ProtoSessionMessageType::FnfMsg => {
                // Simply send the message to the application
                self.send_message_to_app(message).await
            }
            ProtoSessionMessageType::FnfReliable => {
                // Send an ack back as reply and forward the incoming message to the app
                // Create ack message
                let src = message.message.get_source();
                let slim_header = Some(SlimHeader::new(
                    &self.state.source,
                    src.agent_type(),
                    Some(src.agent_id()),
                    Some(
                        SlimHeaderFlags::default()
                            .with_forward_to(message.message.get_incoming_conn()),
                    ),
                ));

                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::SessionFireForget.into(),
                    ProtoSessionMessageType::FnfAck.into(),
                    message.info.id,
                    message_id,
                ));

                let ack =
                    Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

                // Forward the message to the app
                self.send_message_to_app(message).await?;

                // Send the ack
                self.state
                    .tx
                    .send_to_slim(Ok(ack))
                    .await
                    .map_err(|e| SessionError::SlimTransmission(e.to_string()))
            }
            ProtoSessionMessageType::FnfAck => {
                // Remove the timer and drop the message
                self.stop_and_remove_timer(message_id)
            }
            ProtoSessionMessageType::ChannelDiscoveryReply => {
                // Handle sticky session discovery
                self.handle_channel_discovery_reply(message).await
            }
            ProtoSessionMessageType::ChannelJoinRequest => {
                // Handle sticky session discovery
                self.handle_channel_join_request(message).await
            }
            ProtoSessionMessageType::ChannelJoinReply => {
                // Handle sticky session discovery reply
                self.handle_channel_join_reply(message).await
            }
            ProtoSessionMessageType::ChannelLeaveRequest
            | ProtoSessionMessageType::ChannelLeaveReply
            | ProtoSessionMessageType::ChannelMlsWelcome
            | ProtoSessionMessageType::ChannelMlsCommit
            | ProtoSessionMessageType::ChannelMlsProposal
            | ProtoSessionMessageType::ChannelMlsAck => {
                // Handle mls stuff
                self.state
                    .channel_endpoint
                    .on_message(message.message)
                    .await?;

                // Flush the sticky buffer if MLS is enabled
                if self.state.channel_endpoint.is_mls_up()? {
                    // If MLS is up, send all buffered messages
                    let messages: Vec<Message> = self.state.sticky_buffer.drain(..).collect();

                    for msg in messages {
                        self.send_message(msg, None).await?;
                    }
                }

                Ok(())
            }
            _ => {
                // Unexpected header
                Err(SessionError::AppTransmission(format!(
                    "invalid session header {}",
                    message.message.get_session_message_type() as u32
                )))
            }
        }
    }

    /// Helper function to send a message to the application.
    /// This is used by both the Fnf and FnfReliable message handlers.
    async fn send_message_to_app(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        self.state
            .tx
            .send_to_app(Ok(message))
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }

    /// Helper function to stop and remove a timer by message ID.
    /// Returns Ok(()) if the timer was found and stopped, or an appropriate error if not.
    fn stop_and_remove_timer(&mut self, message_id: u32) -> Result<(), SessionError> {
        match self.state.timers.remove(&message_id) {
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

/// The interface for the Fire and Forget session
pub(crate) struct FireAndForget<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    common: Common<P, V, T>,
    tx: Sender<InternalMessage>,
    cancellation_token: CancellationToken,
}
impl<P, V, T> FireAndForget<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: FireAndForgetConfiguration,
        session_direction: SessionDirection,
        agent: Agent,
        tx_slim_app: T,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
    ) -> Self {
        let (tx, rx) = mpsc::channel(32);

        // Common session stuff
        let common = Common::new(
            id,
            session_direction,
            SessionConfig::FireAndForget(session_config.clone()),
            agent,
            tx_slim_app.clone(),
            identity_provider,
            identity_verifier,
            session_config.mls_enabled,
            storage_path,
        );

        // Create mls state if needed
        let mls = common
            .mls()
            .map(|mls| MlsState::new(mls).expect("failed to create MLS state"));

        // Create channel endpoint to handle sticky sessions and encryption
        let channel_endpoint = match session_config.initiator {
            true => {
                let cm = ChannelModerator::new(
                    common.source().clone(),
                    common.source().agent_type().clone(),
                    common.source().agent_id_option(),
                    id,
                    ProtoSessionType::SessionFireForget,
                    60,
                    Duration::from_secs(1),
                    mls,
                    tx_slim_app.clone(),
                );
                ChannelEndpoint::ChannelModerator(cm)
            }
            false => {
                let cp = ChannelParticipant::new(
                    common.source().clone(),
                    common.source().agent_type().clone(),
                    common.source().agent_id_option(),
                    id,
                    ProtoSessionType::SessionFireForget,
                    60,
                    Duration::from_secs(1),
                    mls,
                    tx_slim_app.clone(),
                );
                ChannelEndpoint::ChannelParticipant(cp)
            }
        };

        // FireAndForget internal state
        let state = FireAndForgetState {
            session_id: id,
            source: common.source().clone(),
            tx: tx_slim_app.clone(),
            config: session_config,
            timers: HashMap::new(),
            sticky_name: None,
            sticky_connection: None,
            sticky_session_status: StickySessionStatus::Uninitialized,
            sticky_buffer: VecDeque::new(),
            channel_endpoint,
        };

        // Cancellation token
        let cancellation_token = CancellationToken::new();

        // Create the processor
        let processor =
            FireAndForgetProcessor::new(state, tx.clone(), rx, cancellation_token.clone());

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        FireAndForget {
            common,
            tx,
            cancellation_token,
        }
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for FireAndForget<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
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
            SessionConfig::FireAndForget(config) => config.clone(),
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

    fn source(&self) -> &Agent {
        self.common.source()
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
}

impl<P, V, T> Drop for FireAndForget<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        // Signal the processor to stop
        self.cancellation_token.cancel();
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for FireAndForget<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx
            .send(InternalMessage::OnMessage { message, direction })
            .await
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
        channel_endpoint::handle_channel_discovery_message, testutils::MockTransmitter,
        transmitter::Transmitter,
    };
    use slim_datapath::{
        api::ProtoMessage,
        messages::{Agent, AgentType},
    };

    #[tokio::test]
    async fn test_fire_and_forget_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
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
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
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
        header.set_session_message_type(ProtoSessionMessageType::FnfMsg);

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
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let mut message = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;
        header.message_id = 12345;
        header.set_session_message_type(ProtoSessionMessageType::FnfReliable);

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

        let msg = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let header = msg.get_session_header();
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::FnfAck
        );
        assert_eq!(header.get_message_id(), 12345);
    }

    #[tokio::test]
    async fn test_fire_and_forget_timers_until_error() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                sticky: false,
                mls_enabled: false,
                initiator: true,
            },
            SessionDirection::Bidirectional,
            source.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
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
        header.set_session_message_type(ProtoSessionMessageType::FnfReliable);
        header.set_session_type(ProtoSessionType::SessionFireForget);

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
    async fn test_fire_and_forget_timers_and_ack() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let tx_sender = MockTransmitter {
            tx_app: tx_app_sender,
            tx_slim: tx_slim_sender,
        };

        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let tx_receiver = MockTransmitter {
            tx_app: tx_app_receiver,
            tx_slim: tx_slim_receiver,
        };

        let local = Agent::from_strings("cisco", "default", "local_agent", 0);
        let remote = Agent::from_strings("cisco", "default", "remote_agent", 0);

        let session_sender = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                sticky: false,
                mls_enabled: false,
                initiator: true,
            },
            SessionDirection::Bidirectional,
            local.clone(),
            tx_sender,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        // this can be a standard fnf session
        let session_recv = FireAndForget::new(
            0,
            FireAndForgetConfiguration::default(),
            SessionDirection::Bidirectional,
            remote.clone(),
            tx_receiver,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
        );

        let mut message = ProtoMessage::new_publish(
            &local,
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.set_session_id(0);
        header.set_session_type(ProtoSessionType::SessionFireForget);
        header.set_session_message_type(ProtoSessionMessageType::FnfReliable);

        let res = session_sender
            .on_message(
                SessionMessage::from(message.clone()),
                MessageDirection::South,
            )
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
        let ack = rx_slim_receiver
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        let header = ack.get_session_header();
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::FnfAck
        );

        // Check that the ack is sent back to the sender
        assert_eq!(message.get_source(), ack.get_name_as_agent());

        // deliver the ack to the sender
        let res = session_sender
            .on_message(SessionMessage::from(ack.clone()), MessageDirection::North)
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let tx = MockTransmitter { tx_app, tx_slim };

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        {
            let _session = FireAndForget::new(
                0,
                FireAndForgetConfiguration::default(),
                SessionDirection::Bidirectional,
                source.clone(),
                tx,
                SharedSecret::new("a", "group"),
                SharedSecret::new("a", "group"),
                std::path::PathBuf::from("/tmp/test_session"),
            );
        }

        // sleep for a bit to let the session drop
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // // check that the session is closed
        // assert!(logs_contain(
        //     "fire and forget channel closed, exiting processor loop"
        // ));
    }

    async fn template_test_fire_and_forget_sticky_session(mls_enabled: bool) {
        let (sender_tx_slim, mut sender_rx_slim) = tokio::sync::mpsc::channel(1);
        let (sender_tx_app, _sender_rx_app) = tokio::sync::mpsc::channel(1);

        let sender_tx = Transmitter {
            slim_tx: sender_tx_slim,
            app_tx: sender_tx_app,
            interceptors: Arc::new(RwLock::new(Vec::new())),
        };

        let (receiver_tx_slim, mut receiver_rx_slim) = tokio::sync::mpsc::channel(1);
        let (receiver_tx_app, mut receiver_rx_app) = tokio::sync::mpsc::channel(1);

        let receiver_tx = Transmitter {
            slim_tx: receiver_tx_slim,
            app_tx: receiver_tx_app,
            interceptors: Arc::new(RwLock::new(Vec::new())),
        };

        let local = Agent::from_strings("cisco", "default", "local_agent", 0);
        let remote = Agent::from_strings("cisco", "default", "remote_agent", 0);

        let sender_session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                sticky: true,
                mls_enabled,
                initiator: true,
            },
            SessionDirection::Bidirectional,
            local.clone(),
            sender_tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_sender"),
        );

        let receiver_session = FireAndForget::new(
            0,
            FireAndForgetConfiguration {
                timeout: Some(Duration::from_millis(500)),
                max_retries: Some(5),
                sticky: false,
                mls_enabled,
                initiator: false,
            },
            SessionDirection::Bidirectional,
            remote.clone(),
            receiver_tx,
            SharedSecret::new("b", "group"),
            SharedSecret::new("b", "group"),
            std::path::PathBuf::from("/tmp/test_receiver"),
        );

        // Create a message to send
        let mut message = ProtoMessage::new_publish(
            &local,
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.set_session_id(0);
        header.set_session_message_type(ProtoSessionMessageType::FnfReliable);

        // set a fake incoming connection id
        let slim_header = message.get_slim_header_mut();
        slim_header.set_incoming_conn(Some(0));

        // Send the message
        sender_session
            .on_message(
                SessionMessage::from(message.clone()),
                MessageDirection::South,
            )
            .await
            .expect("failed to send message");

        // We should now get a sticky session discovery message
        let mut msg = sender_rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // Set fake incoming connection id
        msg.set_incoming_conn(Some(0));

        let header = msg.get_session_header_mut();
        header.set_session_message_type(ProtoSessionMessageType::ChannelDiscoveryRequest);

        // assert something
        assert_eq!(
            header.session_message_type(),
            ProtoSessionMessageType::ChannelDiscoveryRequest,
        );

        assert_eq!(msg.get_session_type(), ProtoSessionType::SessionFireForget);

        // create a discovery reply message. this is normally originated by the session layer
        let mut discovery_reply = handle_channel_discovery_message(
            &msg,
            &remote,
            receiver_session.id(),
            ProtoSessionType::SessionFireForget,
        );
        discovery_reply.set_incoming_conn(Some(0));

        // Pass discovery reply message to the sender session
        sender_session
            .on_message(
                SessionMessage::from(discovery_reply),
                MessageDirection::North,
            )
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
            ProtoSessionMessageType::ChannelJoinRequest
        );

        assert_eq!(header.session_type(), ProtoSessionType::SessionFireForget);

        // Set a fake incoming connection id
        msg.set_incoming_conn(Some(0));

        // Pass the channel join request message to the receiver session
        receiver_session
            .on_message(SessionMessage::from(msg.clone()), MessageDirection::North)
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
            ProtoSessionMessageType::ChannelJoinReply
        );

        assert_eq!(header.session_type(), ProtoSessionType::SessionFireForget);

        // Pass the channel join reply message to the sender session
        msg.set_incoming_conn(Some(0));
        sender_session
            .on_message(SessionMessage::from(msg), MessageDirection::North)
            .await
            .expect("failed to handle channel join reply");

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
                ProtoSessionMessageType::ChannelMlsWelcome
            );

            assert_eq!(header.session_type(), ProtoSessionType::SessionFireForget);

            // Set a fake incoming connection id
            msg.set_incoming_conn(Some(0));

            // Pass the MlsWelcome message to the receiver session
            receiver_session
                .on_message(SessionMessage::from(msg), MessageDirection::North)
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
                ProtoSessionMessageType::ChannelMlsAck
            );

            assert_eq!(header.session_type(), ProtoSessionType::SessionFireForget);

            // Send the ack to the sender session
            msg.set_incoming_conn(Some(0));
            sender_session
                .on_message(SessionMessage::from(msg), MessageDirection::North)
                .await
                .expect("failed to handle mls ack");

            // Now we should get the original message
            let mut msg = sender_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            let header = msg.get_session_header();

            assert_eq!(
                header.session_message_type(),
                ProtoSessionMessageType::FnfReliable
            );

            assert_eq!(header.session_type(), ProtoSessionType::SessionFireForget);

            // As MLS is enabled, the payload should be encrypted
            tracing::info!(
                "Checking if payload is encrypted {}",
                msg.get_payload().unwrap().blob.len()
            );
            assert!(!msg.get_payload().unwrap().blob.is_empty());
            assert_ne!(msg.get_payload(), message.get_payload());

            // Pass message to the receiver session
            msg.set_incoming_conn(Some(0));
            receiver_session
                .on_message(SessionMessage::from(msg), MessageDirection::North)
                .await
                .expect("failed to handle message");

            // Get message from the receiver app
            let msg = receiver_rx_app
                .recv()
                .await
                .expect("no message received")
                .expect("error");

            // Check that the payload is decrypted
            assert_eq!(msg.message.get_payload(), message.get_payload());
        } else {
            // The sender session should now send the original message to the receiver
            let mut msg = sender_rx_slim
                .recv()
                .await
                .expect("no message received")
                .expect("error");
            let header = msg.get_session_header();
            assert_eq!(
                header.session_message_type(),
                ProtoSessionMessageType::FnfReliable
            );

            msg.set_incoming_conn(Some(0));

            assert_eq!(msg.get_payload(), message.get_payload());
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_fire_and_forget_sticky_session_no_mls() {
        template_test_fire_and_forget_sticky_session(false).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_fire_and_forget_sticky_session_mls() {
        template_test_fire_and_forget_sticky_session(true).await;
    }
}
