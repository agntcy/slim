// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{collections::HashMap, time::Duration};

use parking_lot::Mutex;
use tokio::sync::{self, oneshot};
// Third-party crates
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, Content, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    MessageDirection, SessionError, Transmitter,
    common::SessionMessage,
    completion_handle::CompletionHandle,
    controller_sender::ControllerSender,
    session_builder::{ForController, SessionBuilder},
    session_config::SessionConfig,
    session_settings::SessionSettings,
    traits::{MessageHandler, ProcessingState},
};

pub struct SessionController {
    /// session id
    pub(crate) id: u32,

    /// local name
    pub(crate) source: Name,

    /// group or remote endpoint name
    pub(crate) destination: Name,

    /// session config
    pub(crate) config: SessionConfig,

    /// channel to send messages to the processing loop
    tx_controller: sync::mpsc::Sender<SessionMessage>,

    /// use in drop implementation to gracefully close the processing loop
    pub(crate) cancellation_token: CancellationToken,

    /// handle for the processing loop
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SessionController {
    /// Returns a new SessionBuilder for constructing a SessionController
    pub fn builder<P, V>() -> SessionBuilder<P, V, ForController>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        SessionBuilder::for_controller()
    }

    /// Internal constructor for the builder to use
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts<I, P, V>(
        id: u32,
        source: Name,
        destination: Name,
        config: SessionConfig,
        settings: SessionSettings<P, V>,
        tx: sync::mpsc::Sender<SessionMessage>,
        rx: sync::mpsc::Receiver<SessionMessage>,
        inner: I,
    ) -> Self
    where
        I: MessageHandler + Send + Sync + 'static,
        P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
        V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
    {
        // Spawn the processing loop
        let cancellation_token = CancellationToken::new();

        // setup tracing context
        let span = tracing::debug_span!(
            "session_controller_processing_loop",
            session_id = id,
            source = %source,
            destination = %destination,
            session_type = ?config.session_type
        );

        let handle = tokio::spawn(
            Self::processing_loop(inner, rx, cancellation_token.clone(), settings).instrument(span),
        );

        Self {
            id,
            source,
            destination,
            config,
            tx_controller: tx,
            cancellation_token,
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Internal processing loop that handles messages with mutable access
    fn enter_draining_state<P, V>(
        shutdown_deadline: &mut std::pin::Pin<&mut tokio::time::Sleep>,
        settings: &SessionSettings<P, V>,
    ) where
        P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
        V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
    {
        let shutdown_timeout = settings
            .graceful_shutdown_timeout
            .unwrap_or(Duration::from_secs(60));
        shutdown_deadline
            .as_mut()
            .reset(tokio::time::Instant::now() + shutdown_timeout);
    }

    async fn processing_loop<P, V>(
        mut inner: impl MessageHandler + 'static,
        mut rx: sync::mpsc::Receiver<SessionMessage>,
        cancellation_token: CancellationToken,
        settings: SessionSettings<P, V>,
    ) where
        P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
        V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
    {
        // Start with an infinite timeout (will be updated on graceful shutdown)
        let mut shutdown_deadline = std::pin::pin!(tokio::time::sleep(Duration::MAX));
        // Pin the cancellation token
        // let mut cancellation_token = std::pin::pin!(cancellation_token.cancelled());

        // Init the inner components
        if let Err(e) = inner.init().await {
            tracing::error!(error=%e, "Error during initialization of session");
        }

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled(), if inner.processing_state() == ProcessingState::Active => {
                    // Update the timeout to the configured grace period
                    let shutdown_timeout = settings.graceful_shutdown_timeout
                        .unwrap_or(Duration::from_secs(60)); // Default 60 seconds if not configured

                    // Finish any ongoing processing before starting drain
                    debug!("consuming pending messages before entering draining state");
                    while let Ok(msg) = rx.try_recv() {
                        if let Err(e) = inner.on_message(msg).await {
                            tracing::error!(error=%e, "Error processing message during draining. Close immediately.");
                            break;
                        }
                    }

                    // Send drain to message to the inner to notify the beginning of the drain
                    if let Err(e) = inner.on_message(SessionMessage::StartDrain {
                        grace_period: shutdown_timeout
                    }).await {
                        tracing::error!(error=%e, "Error during start drain");
                        break;
                    }

                    Self::enter_draining_state(&mut shutdown_deadline, &settings);

                    debug!("cancellation requested, entering draining state");
                }
                _ = &mut shutdown_deadline => {
                    debug!("graceful shutdown timeout reached, forcing exit");
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(session_message) => {
                            let draining = inner.processing_state() == ProcessingState::Draining;

                            // if draining and message is sent by the application, reject it
                            if draining && matches!(session_message, SessionMessage::OnMessage { direction: MessageDirection::South, .. }) {
                                tracing::debug!("session is draining, rejecting new messages from application");
                                if let SessionMessage::OnMessage { ack_tx: Some(ack_tx), .. } = session_message {
                                    let _ = ack_tx.send(Err(SessionError::Processing("Session is draining, cannot accept new messages".to_string())));
                                }
                                continue;
                            }

                            if let Err(e) = inner.on_message(session_message).await {
                                tracing::error!(
                                    error=%e,
                                    "Error processing message{}",
                                    if draining { " during graceful shutdown" } else { "" }
                                );
                                if draining {
                                    debug!("Exiting processing loop due to error while draining");
                                    break;
                                }
                            } else {
                                // If we were active before processing and the handler switched to draining,
                                // start (or reset) the graceful shutdown deadline just like on cancellation.
                                if !draining && inner.processing_state() == ProcessingState::Draining {
                                    debug!("internal component requested draining, entering draining state");
                                    Self::enter_draining_state(&mut shutdown_deadline, &settings);
                                }
                            }
                        }
                        None => {
                            debug!("Session channel closed, no more messages can arrive - exiting processing loop");
                            break;
                        }
                    }
                }
            }

            // If we are in draining state and the inner component does not require drain, exit
            if inner.processing_state() == ProcessingState::Draining && !inner.needs_drain() {
                debug!("draining complete, exiting processing loop");
                break;
            }
        }

        // Perform final shutdown
        if let Err(e) = inner.on_shutdown().await {
            tracing::error!(error=%e, "Error during shutdown of session");
        }
    }

    /// getters
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn source(&self) -> &Name {
        &self.source
    }

    pub fn dst(&self) -> &Name {
        &self.destination
    }

    pub fn session_type(&self) -> ProtoSessionType {
        self.config.session_type
    }

    pub fn metadata(&self) -> HashMap<String, String> {
        self.config.metadata.clone()
    }

    pub fn session_config(&self) -> SessionConfig {
        self.config.clone()
    }

    pub fn is_initiator(&self) -> bool {
        self.config.initiator
    }

    async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
        ack_tx: Option<oneshot::Sender<Result<(), SessionError>>>,
    ) -> Result<(), SessionError> {
        self.tx_controller
            .send(SessionMessage::OnMessage {
                message,
                direction,
                ack_tx,
            })
            .await
            .map_err(|e| {
                SessionError::Processing(format!(
                    "Failed to send message to session controller: {}",
                    e
                ))
            })
    }

    /// Send a message to the controller for processing
    pub async fn on_message_from_app(
        &self,
        message: Message,
    ) -> Result<CompletionHandle, SessionError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.on_message(message, MessageDirection::South, Some(ack_tx))
            .await?;

        let ret = CompletionHandle::from_oneshot_receiver(ack_rx);

        Ok(ret)
    }

    /// Send a message to the controller for processing
    pub async fn on_message_from_slim(&self, message: Message) -> Result<(), SessionError> {
        self.on_message(message, MessageDirection::North, None)
            .await
    }

    pub fn close(&self) -> Result<tokio::task::JoinHandle<()>, SessionError> {
        self.cancellation_token.cancel();

        self.handle
            .lock()
            .take()
            .ok_or(SessionError::Generic("Session already closed".to_string()))
    }

    pub async fn publish_message(
        &self,
        message: Message,
    ) -> Result<CompletionHandle, SessionError> {
        self.on_message_from_app(message).await
    }

    /// Publish a message to a specific connection (forward_to)
    pub async fn publish_to(
        &self,
        name: &Name,
        forward_to: u64,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default().with_forward_to(forward_to),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message to a specific app name
    pub async fn publish(
        &self,
        name: &Name,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default(),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message with specific flags
    pub async fn publish_with_flags(
        &self,
        name: &Name,
        flags: SlimHeaderFlags,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, SessionError> {
        let ct = payload_type.unwrap_or_else(|| "msg".to_string());

        let mut msg = Message::builder()
            .source(self.source().clone())
            .destination(name.clone())
            .identity("")
            .flags(flags)
            .session_type(self.session_type())
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(self.id())
            .message_id(rand::random::<u32>()) // this will be changed by the session itself
            .application_payload(&ct, blob)
            .build_publish()
            .map_err(|e| SessionError::Processing(e.to_string()))?;
        if let Some(map) = metadata
            && !map.is_empty()
        {
            msg.set_metadata_map(map);
        }

        // southbound=true means towards slim
        self.publish_message(msg).await
    }

    /// Creates a discovery request message with minimum required information
    fn create_discovery_request(&self, destination: &Name) -> Result<Message, SessionError> {
        let payload = CommandPayload::builder()
            .discovery_request(None)
            .as_content();
        Message::builder()
            .source(self.source().clone())
            .destination(destination.clone())
            .identity("")
            .session_type(self.session_type())
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(self.id())
            .message_id(rand::random::<u32>())
            .payload(payload)
            .build_publish()
            .map_err(|e| SessionError::Processing(e.to_string()))
    }

    pub(crate) async fn invite_participant_internal(
        &self,
        destination: &Name,
    ) -> Result<CompletionHandle, SessionError> {
        let msg = self.create_discovery_request(destination)?;
        self.publish_message(msg).await
    }

    pub async fn invite_participant(
        &self,
        destination: &Name,
    ) -> Result<CompletionHandle, SessionError> {
        match self.session_type() {
            ProtoSessionType::PointToPoint => Err(SessionError::Processing(
                "cannot invite participant to point-to-point session".into(),
            )),
            ProtoSessionType::Multicast => {
                if !self.is_initiator() {
                    return Err(SessionError::Processing(
                        "cannot invite participant to this session session".into(),
                    ));
                }
                self.invite_participant_internal(destination).await
            }
            _ => Err(SessionError::Processing("unexpected session type".into())),
        }
    }

    pub async fn remove_participant(
        &self,
        destination: &Name,
    ) -> Result<CompletionHandle, SessionError> {
        match self.session_type() {
            ProtoSessionType::PointToPoint => Err(SessionError::Processing(
                "cannot remove participant to point-to-point session".into(),
            )),
            ProtoSessionType::Multicast => {
                if !self.is_initiator() {
                    return Err(SessionError::Processing(
                        "cannot remove participant from this session session".into(),
                    ));
                }
                let msg = Message::builder()
                    .source(self.source().clone())
                    .destination(destination.clone().with_id(Name::NULL_COMPONENT))
                    .identity("")
                    .session_type(ProtoSessionType::Multicast)
                    .session_message_type(ProtoSessionMessageType::LeaveRequest)
                    .session_id(self.id())
                    .message_id(rand::random::<u32>())
                    .payload(CommandPayload::builder().leave_request(None).as_content())
                    .build_publish()
                    .map_err(|e| SessionError::Processing(e.to_string()))?;
                self.publish_message(msg).await
            }
            _ => Err(SessionError::Processing("unexpected session type".into())),
        }
    }
}

impl Drop for SessionController {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
    }
}

pub fn handle_channel_discovery_message(
    message: &Message,
    app_name: &Name,
    session_id: u32,
    session_type: ProtoSessionType,
) -> Result<Message, SessionError> {
    let destination = message.get_source();

    // the destination of the discovery message may be different from the name of
    // application itself. This can happen if the application subscribes to multiple
    // service names. So we can reply using as a source the destination name of
    // the discovery message but setting the application id

    let mut source = message.get_dst();
    source.set_id(app_name.id());
    let msg_id = message.get_id();

    let slim_header = SlimHeader::new(
        &source,
        &destination,
        "", // the identity will be added by the identity interceptor
        Some(SlimHeaderFlags::default().with_forward_to(message.get_incoming_conn())),
    );

    debug!("Received discovery request, reply to the msg source");

    Message::builder()
        .with_slim_header(slim_header)
        .session_type(session_type)
        .session_message_type(ProtoSessionMessageType::DiscoveryReply)
        .session_id(session_id)
        .message_id(msg_id)
        .payload(CommandPayload::builder().discovery_reply().as_content())
        .build_publish()
        .map_err(|e| SessionError::Processing(e.to_string()))
}

pub(crate) struct SessionControllerCommon<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// common session fields
    pub(crate) settings: SessionSettings<P, V>,

    /// sender for command messages
    pub(crate) sender: ControllerSender,

    /// processing state
    pub(crate) processing_state: ProcessingState,
}

impl<P, V> SessionControllerCommon<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(settings: SessionSettings<P, V>) -> Self {
        // create the controller sender
        let controller_sender = ControllerSender::new(
            settings.config.get_timer_settings(),
            settings.source.clone(),
            // send messages to slim/app
            settings.tx.clone(),
            // send signal to the controller
            settings.tx_session.clone(),
        );

        SessionControllerCommon {
            settings,
            sender: controller_sender,
            processing_state: ProcessingState::Active,
        }
    }

    /// internal and helper functions
    pub(crate) async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.settings.tx.send_to_slim(Ok(message)).await
    }

    /// Send control message without creating ack channel (for internal use by moderator)
    pub(crate) async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
    }

    pub(crate) async fn set_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::builder()
            .source(self.settings.source.clone())
            .destination(name.clone())
            .flags(SlimHeaderFlags::default().with_recv_from(conn))
            .build_subscribe()
            .unwrap();

        self.send_to_slim(route).await
    }

    pub(crate) async fn delete_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::builder()
            .source(self.settings.source.clone())
            .destination(name.clone())
            .flags(SlimHeaderFlags::default().with_recv_from(conn))
            .build_unsubscribe()
            .unwrap();

        self.send_to_slim(route).await
    }

    pub(crate) fn create_control_message(
        &mut self,
        dst: &Name,
        message_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Content,
        broadcast: bool,
    ) -> Result<Message, SessionError> {
        let mut builder = Message::builder()
            .source(self.settings.source.clone())
            .destination(dst.clone())
            .identity("")
            .session_type(self.settings.config.session_type)
            .session_message_type(message_type)
            .session_id(self.settings.id)
            .message_id(message_id)
            .payload(payload);

        if broadcast {
            builder = builder.fanout(256);
        }

        builder
            .build_publish()
            .map_err(|e| SessionError::Processing(e.to_string()))
    }

    /// Send control message without creating ack channel (for internal use by moderator)
    pub(crate) async fn send_control_message(
        &mut self,
        dst: &Name,
        message_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Content,
        metadata: Option<HashMap<String, String>>,
        broadcast: bool,
    ) -> Result<(), SessionError> {
        let mut msg =
            self.create_control_message(dst, message_type, message_id, payload, broadcast)?;
        if let Some(m) = metadata {
            msg.set_metadata_map(m);
        }
        self.send_with_timer(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test: internal draining transition triggered by a leave request.
    // This test sends a LeaveRequest into a multicast participant session and then
    // verifies (indirectly) that subsequent messages are still accepted while the
    // session is transitioning, indicating that graceful draining has begun.
    // Removed broken test_internal_draining_via_leave_request (incompatible mock trait implementation)

    use crate::transmitter::SessionTransmitter;
    use slim_auth::shared_secret::SharedSecret;

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    const SHARED_SECRET: &str = "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas";

    /// Test helper to create a SessionController with common setup
    struct SessionControllerTestBuilder {
        session_id: u32,
        source: Name,
        destination: Name,
        session_type: ProtoSessionType,
        mls_enabled: bool,
        initiator: bool,
        max_retries: Option<u32>,
        interval: Option<Duration>,
        metadata: HashMap<String, String>,
        graceful_shutdown_timeout: Option<Duration>,
    }

    impl SessionControllerTestBuilder {
        #[allow(dead_code)]
        fn new() -> Self {
            Self {
                session_id: 10,
                source: Name::from_strings(["org", "ns", "source"]).with_id(1),
                destination: Name::from_strings(["org", "ns", "dest"]).with_id(2),
                session_type: ProtoSessionType::PointToPoint,
                mls_enabled: false,
                initiator: true,
                max_retries: Some(5),
                interval: Some(Duration::from_millis(200)),
                metadata: HashMap::new(),
                graceful_shutdown_timeout: Some(Duration::from_secs(10)),
            }
        }

        fn with_session_id(mut self, id: u32) -> Self {
            self.session_id = id;
            self
        }

        #[allow(dead_code)]
        fn with_source(mut self, source: Name) -> Self {
            self.source = source;
            self
        }

        #[allow(dead_code)]
        fn with_destination(mut self, destination: Name) -> Self {
            self.destination = destination;
            self
        }

        fn with_session_type(mut self, session_type: ProtoSessionType) -> Self {
            self.session_type = session_type;
            self
        }

        fn with_mls_enabled(mut self, enabled: bool) -> Self {
            self.mls_enabled = enabled;
            self
        }

        fn with_initiator(mut self, initiator: bool) -> Self {
            self.initiator = initiator;
            self
        }

        fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
            self.metadata = metadata;
            self
        }

        fn with_graceful_shutdown_timeout(mut self, timeout: Duration) -> Self {
            self.graceful_shutdown_timeout = Some(timeout);
            self
        }

        fn build(
            self,
        ) -> (
            SessionController,
            tokio::sync::mpsc::Receiver<Result<Message, slim_datapath::Status>>,
            tokio::sync::mpsc::UnboundedReceiver<Result<Message, SessionError>>,
        ) {
            let config = SessionConfig {
                session_type: self.session_type,
                max_retries: self.max_retries,
                interval: self.interval,
                mls_enabled: self.mls_enabled,
                initiator: self.initiator,
                metadata: self.metadata,
            };

            let (tx_slim, rx_slim) = tokio::sync::mpsc::channel(10);
            let (tx_app, rx_app) = tokio::sync::mpsc::unbounded_channel();
            let (tx_session_layer, _rx_session_layer) = tokio::sync::mpsc::channel(10);

            let tx = SessionTransmitter::new(tx_slim, tx_app);

            let storage_path =
                std::path::PathBuf::from(format!("/tmp/test_session_{}", rand::random::<u64>()));

            let controller = SessionController::builder()
                .with_id(self.session_id)
                .with_source(self.source.clone())
                .with_destination(self.destination.clone())
                .with_config(config)
                .with_identity_provider(SharedSecret::new("test", SHARED_SECRET))
                .with_identity_verifier(SharedSecret::new("test", SHARED_SECRET))
                .with_storage_path(storage_path)
                .with_tx(tx)
                .with_tx_to_session_layer(tx_session_layer)
                .ready()
                .expect("failed to validate builder")
                .build()
                .expect("failed to build controller");

            (controller, rx_slim, rx_app)
        }
    }

    #[tokio::test]
    async fn test_session_controller_getters() {
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());

        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_id(42)
            .with_session_type(ProtoSessionType::Multicast)
            .with_mls_enabled(true)
            .with_metadata(metadata)
            .build();

        assert_eq!(controller.id(), 42);
        assert_eq!(
            controller.source(),
            &Name::from_strings(["org", "ns", "source"]).with_id(1)
        );
        assert_eq!(
            controller.dst(),
            &Name::from_strings(["org", "ns", "dest"]).with_id(2)
        );
        assert_eq!(controller.session_type(), ProtoSessionType::Multicast);
        assert!(controller.is_initiator());
        assert_eq!(
            controller.metadata().get("key1"),
            Some(&"value1".to_string())
        );

        let retrieved_config = controller.session_config();
        assert_eq!(retrieved_config.session_type, ProtoSessionType::Multicast);
        assert_eq!(retrieved_config.max_retries, Some(5));
    }

    #[tokio::test]
    async fn test_publish_basic() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new().build();

        let target_name = Name::from_strings(["org", "ns", "target"]);
        let payload = b"Hello World".to_vec();

        controller
            .publish(
                &target_name,
                payload.clone(),
                Some("test-type".to_string()),
                None,
            )
            .await
            .expect("publish should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_publish_to_specific_connection() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .build();

        let target_name = Name::from_strings(["org", "ns", "target"]);
        let payload = b"Hello to specific connection".to_vec();
        let connection_id = 123u64;

        controller
            .publish_to(
                &target_name,
                connection_id,
                payload.clone(),
                Some("test-type".to_string()),
                None,
            )
            .await
            .expect("publish_to should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_publish_with_metadata() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .build();

        let target_name = Name::from_strings(["org", "ns", "target"]);
        let payload = b"Hello with metadata".to_vec();

        let mut metadata = HashMap::new();
        metadata.insert("custom_key".to_string(), "custom_value".to_string());

        controller
            .publish(
                &target_name,
                payload.clone(),
                Some("test-type".to_string()),
                Some(metadata),
            )
            .await
            .expect("publish with metadata should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_invite_participant_in_multicast() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .build();

        let participant = Name::from_strings(["org", "ns", "participant"]);

        controller
            .invite_participant(&participant)
            .await
            .expect("invite should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_invite_participant_not_initiator_error() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .with_initiator(false)
            .build();

        let participant = Name::from_strings(["org", "ns", "new_participant"]);

        let result = controller.invite_participant(&participant).await;
        assert!(result.is_err());
        if let Err(SessionError::Processing(msg)) = result {
            assert!(msg.contains("cannot invite participant"));
        } else {
            panic!("Expected SessionError::Processing");
        }
    }

    #[tokio::test]
    async fn test_invite_participant_p2p_error() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::PointToPoint)
            .build();

        let participant = Name::from_strings(["org", "ns", "participant"]);

        let result = controller.invite_participant(&participant).await;
        assert!(result.is_err());
        if let Err(SessionError::Processing(msg)) = result {
            assert!(msg.contains("cannot invite participant to point-to-point"));
        } else {
            panic!("Expected SessionError::Processing");
        }
    }

    #[tokio::test]
    async fn test_remove_participant_in_multicast() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .build();

        let participant = Name::from_strings(["org", "ns", "participant"]);

        controller
            .remove_participant(&participant)
            .await
            .expect("remove should succeed");

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_remove_participant_not_initiator_error() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .with_initiator(false)
            .build();

        let participant = Name::from_strings(["org", "ns", "participant"]);

        let result = controller.remove_participant(&participant).await;
        assert!(result.is_err());
        if let Err(SessionError::Processing(msg)) = result {
            assert!(msg.contains("cannot remove participant"));
        } else {
            panic!("Expected SessionError::Processing");
        }
    }

    #[tokio::test]
    async fn test_remove_participant_p2p_error() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::PointToPoint)
            .build();

        let participant = Name::from_strings(["org", "ns", "participant"]);

        let result = controller.remove_participant(&participant).await;
        assert!(result.is_err());
        if let Err(SessionError::Processing(msg)) = result {
            assert!(msg.contains("cannot remove participant to point-to-point"));
        } else {
            panic!("Expected SessionError::Processing");
        }
    }

    #[test]
    fn test_handle_channel_discovery_message() {
        let app_name = Name::from_strings(["org", "ns", "app"]).with_id(100);
        let session_id = 42;

        let discovery_request = Message::builder()
            .source(Name::from_strings(["org", "ns", "requester"]).with_id(1))
            .destination(Name::from_strings(["org", "ns", "service"]))
            .identity("")
            .incoming_conn(999)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(session_id)
            .message_id(123)
            .payload(
                CommandPayload::builder()
                    .discovery_request(None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let response = handle_channel_discovery_message(
            &discovery_request,
            &app_name,
            session_id,
            ProtoSessionType::Multicast,
        )
        .expect("should create discovery response");

        assert_eq!(
            response.get_session_message_type(),
            ProtoSessionMessageType::DiscoveryReply
        );
        assert_eq!(response.get_session_header().get_session_id(), session_id);
        assert_eq!(response.get_id(), 123);
        assert_eq!(
            response.get_dst(),
            Name::from_strings(["org", "ns", "requester"]).with_id(1)
        );
        assert_eq!(response.get_slim_header().get_forward_to(), Some(999));
    }

    #[tokio::test]
    async fn test_controller_drop_cancels_processing() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new().build();

        let token = controller.cancellation_token.clone();
        assert!(!token.is_cancelled());

        drop(controller);

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn test_close_success() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_graceful_shutdown_timeout(std::time::Duration::from_secs(2))
            .build();

        let token = controller.cancellation_token.clone();
        assert!(!token.is_cancelled());

        let handle = controller.close();
        assert!(handle.is_ok(), "got error {}", handle.unwrap_err());
        assert!(token.is_cancelled());

        // Wait for the handle to complete
        handle
            .unwrap()
            .await
            .expect("processing task should complete");
    }

    #[tokio::test]
    async fn test_close_already_closed() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new().build();

        // Close once - should succeed
        let handle = controller.close();
        assert!(handle.is_ok());
        handle
            .unwrap()
            .await
            .expect("processing task should complete");

        // Close again - should fail with appropriate error
        let result = controller.close();
        assert!(result.is_err());
        match result {
            Err(SessionError::Generic(msg)) => {
                assert_eq!(msg, "Session already closed");
            }
            _ => panic!("Expected SessionError::Generic with 'Session already closed' message"),
        }
    }

    #[tokio::test]
    async fn test_close_cancels_token_immediately() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new().build();

        let token = controller.cancellation_token.clone();

        // Verify token is not cancelled before close
        assert!(!token.is_cancelled());

        // Close returns immediately after cancelling token
        let handle = controller.close();
        assert!(handle.is_ok());

        // Token should be cancelled immediately
        assert!(token.is_cancelled());

        // Wait for processing to complete
        handle.unwrap().await.expect("processing should complete");
    }

    #[tokio::test]
    async fn test_on_message_direction_north() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new().build();

        let test_message = Message::builder()
            .source(controller.dst().clone())
            .destination(controller.source().clone())
            .identity("")
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(controller.id())
            .message_id(1)
            .application_payload("test", b"test data".to_vec())
            .build_publish()
            .unwrap();

        let result = controller.on_message_from_slim(test_message).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_discovery_request() {
        let (controller, _rx_slim, _rx_app) = SessionControllerTestBuilder::new()
            .with_session_type(ProtoSessionType::Multicast)
            .build();

        let target = Name::from_strings(["org", "ns", "target"]);
        let discovery_msg = controller
            .create_discovery_request(&target)
            .expect("should create discovery request");

        assert_eq!(discovery_msg.get_source(), *controller.source());
        assert_eq!(discovery_msg.get_dst(), target);
        assert_eq!(
            discovery_msg.get_session_message_type(),
            ProtoSessionMessageType::DiscoveryRequest
        );
        assert_eq!(
            discovery_msg.get_session_header().get_session_id(),
            controller.id()
        );
        assert_eq!(
            discovery_msg.get_session_type(),
            ProtoSessionType::Multicast
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end_p2p() {
        let session_id = 10;
        let moderator_name = Name::from_strings(["org", "ns", "moderator"]).with_id(1);
        let participant_name = Name::from_strings(["org", "ns", "participant"]);
        let participant_name_id = Name::from_strings(["org", "ns", "participant"]).with_id(1);
        let storage_path_moderator = std::path::PathBuf::from("/tmp/test_invite_moderator");
        let storage_path_participant = std::path::PathBuf::from("/tmp/test_invite_participant");

        // create a SessionModerator
        let (tx_slim_moderator, mut rx_slim_moderator) = tokio::sync::mpsc::channel(10);
        let (tx_app_moderator, _rx_app_moderator) = tokio::sync::mpsc::unbounded_channel();
        let (tx_session_layer_moderator, _rx_session_layer_moderator) =
            tokio::sync::mpsc::channel(10);

        let tx_moderator =
            SessionTransmitter::new(tx_slim_moderator.clone(), tx_app_moderator.clone());

        let moderator_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(1000)),
            mls_enabled: true,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let moderator = SessionController::builder()
            .with_id(session_id)
            .with_source(moderator_name.clone())
            .with_destination(participant_name.clone())
            .with_config(moderator_config)
            .with_identity_provider(SharedSecret::new("moderator", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("moderator", SHARED_SECRET))
            .with_storage_path(storage_path_moderator.clone())
            .with_tx(tx_moderator.clone())
            .with_tx_to_session_layer(tx_session_layer_moderator)
            .ready()
            .expect("failed to validate builder")
            .build()
            .unwrap();

        // create a SessionParticipant
        let (tx_slim_participant, mut rx_slim_participant) = tokio::sync::mpsc::channel(10);
        let (tx_app_participant, mut rx_app_participant) = tokio::sync::mpsc::unbounded_channel();
        let (tx_session_layer_participant, _rx_session_layer_participant) =
            tokio::sync::mpsc::channel(10);

        let tx_participant =
            SessionTransmitter::new(tx_slim_participant.clone(), tx_app_participant.clone());

        let participant_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(200)),
            mls_enabled: true,
            initiator: false,
            metadata: std::collections::HashMap::new(),
        };

        let participant = SessionController::builder()
            .with_id(session_id)
            .with_source(participant_name_id.clone())
            .with_destination(moderator_name.clone())
            .with_config(participant_config)
            .with_identity_provider(SharedSecret::new("participant", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("participant", SHARED_SECRET))
            .with_storage_path(storage_path_participant.clone())
            .with_tx(tx_participant.clone())
            .with_tx_to_session_layer(tx_session_layer_participant)
            .ready()
            .expect("failed to validate builder")
            .build()
            .unwrap();

        let completion_handle = moderator
            .invite_participant_internal(&participant_name)
            .await
            .expect("error inviting participant");

        let received_discovery_request =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv())
                .await
                .expect("timeout waiting for discovery request on moderator slim channel")
                .expect("channel closed")
                .expect("error in discovery request");

        assert_eq!(
            received_discovery_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest
        );

        let discovery_msg_id = received_discovery_request.get_id();

        // create a discovery reply and call the on message on the moderator with the reply (direction north)
        let mut discovery_reply = Message::builder()
            .source(participant_name_id.clone())
            .destination(moderator_name.clone())
            .identity("")
            .forward_to(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::DiscoveryReply)
            .session_id(session_id)
            .message_id(discovery_msg_id)
            .payload(CommandPayload::builder().discovery_reply().as_content())
            .build_publish()
            .unwrap();
        discovery_reply
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message_from_slim(discovery_reply)
            .await
            .expect("error processing discovery reply on moderator");

        // check that we get a route for the remote endpoint on slim
        let route = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        // check that the route message type is a subscription, the destination name is remote and the flag recv_from is set to 1
        assert!(route.is_subscribe(), "route should be a subscribe message");
        assert_eq!(route.get_dst(), participant_name_id);
        assert_eq!(route.get_slim_header().get_recv_from(), Some(1));

        // check that a join request is received by slim
        let join_request = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        assert_eq!(
            join_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::JoinRequest
        );
        assert_eq!(join_request.get_dst(), participant_name_id);

        // call the on message on the participant side with the join request (direction north)
        let mut join_request_to_participant = join_request.clone();
        join_request_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message_from_slim(join_request_to_participant)
            .await
            .expect("error processing join request on participant");

        // check that a route for the moderator is generated
        let route = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        assert!(route.is_subscribe(), "route should be a subscribe message");
        assert_eq!(route.get_dst(), moderator_name);
        assert_eq!(route.get_slim_header().get_recv_from(), Some(1));

        // check that a join reply is received by slim on the participant
        let join_reply = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for join reply on participant slim channel")
            .expect("channel closed")
            .expect("error in join reply");

        assert_eq!(
            join_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::JoinReply
        );
        assert_eq!(join_reply.get_dst(), moderator_name);

        // call the on message on the moderator with the reply (direction north)
        let mut join_reply_to_moderator = join_reply.clone();
        join_reply_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message_from_slim(join_reply_to_moderator)
            .await
            .expect("error processing join reply on moderator");

        // check that a welcome message is received by slim on the moderator
        let welcome_message = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for welcome message on moderator slim channel")
            .expect("channel closed")
            .expect("error in welcome message");

        assert_eq!(
            welcome_message.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::GroupWelcome
        );
        assert_eq!(welcome_message.get_dst(), participant_name_id);

        // call the on message on the participant side with the welcome message (direction north)
        let mut welcome_to_participant = welcome_message.clone();
        welcome_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message_from_slim(welcome_to_participant)
            .await
            .expect("error processing welcome message on participant");

        // check that an ack group is received by slim on the participant
        let ack_group = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for ack group on participant slim channel")
            .expect("channel closed")
            .expect("error in ack group");

        assert_eq!(
            ack_group.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::GroupAck
        );
        assert_eq!(ack_group.get_dst(), moderator_name);

        // call the on message on the moderator with the ack (direction north)
        let mut ack_to_moderator = ack_group.clone();
        ack_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message_from_slim(ack_to_moderator)
            .await
            .expect("error processing ack group on moderator");

        // no other message should be sent
        let no_more_moderator = timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;
        assert!(
            no_more_moderator.is_err(),
            "Expected no more messages on moderator slim channel, received {:?}",
            no_more_moderator
                .ok()
                .and_then(|opt| opt)
                .and_then(|res| res.ok())
        );

        let no_more_participant =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant.is_err(),
            "Expected no more messages on participant slim channel"
        );

        // the completion handler should now be complete
        completion_handle.await.expect("error in completion handle");

        // create an application message using the participant name
        let app_data = b"Hello from moderator to participant".to_vec();
        let app_message = Message::builder()
            .source(moderator_name.clone())
            .destination(participant_name.clone())
            .identity("")
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .session_id(session_id)
            .message_id(1)
            .application_payload("test-app-data", app_data.clone())
            .build_publish()
            .unwrap();

        // call on message on the moderator (direction south)
        moderator
            .on_message_from_app(app_message)
            .await
            .expect("error sending application message from moderator");

        // check that message is received from slim with destination equal to participant name id
        let app_msg_to_slim = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for application message on moderator slim channel")
            .expect("channel closed")
            .expect("error in application message");

        assert_eq!(app_msg_to_slim.get_dst(), participant_name_id);
        assert!(
            app_msg_to_slim.is_publish(),
            "message should be a publish message"
        );

        let app_msg_id = app_msg_to_slim.get_id();

        // call the on message on the participant (direction north)
        let mut app_msg_to_participant = app_msg_to_slim.clone();
        app_msg_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message_from_slim(app_msg_to_participant)
            .await
            .expect("error processing application message on participant");

        // check that the message is received by the application
        let app_msg_received = timeout(Duration::from_millis(100), rx_app_participant.recv())
            .await
            .expect("timeout waiting for application message on participant app channel")
            .expect("channel closed")
            .expect("error in application message to app");

        assert_eq!(app_msg_received.get_source(), moderator_name);
        assert!(
            app_msg_received.is_publish(),
            "message should be a publish message"
        );

        let content = app_msg_received
            .get_payload()
            .unwrap()
            .as_application_payload()
            .unwrap()
            .blob
            .clone();
        assert_eq!(content, app_data);

        // check that an ack is sent to slim
        let ack_msg = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for ack on participant slim channel")
            .expect("channel closed")
            .expect("error in ack");

        assert_eq!(
            ack_msg.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MsgAck,
            "message should be an ack"
        );
        assert_eq!(ack_msg.get_dst(), moderator_name);
        assert_eq!(ack_msg.get_id(), app_msg_id);

        // call the on message with the ack on the moderator
        let mut ack_to_moderator = ack_msg.clone();
        ack_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message_from_slim(ack_to_moderator)
            .await
            .expect("error processing ack on moderator");

        // check that no other message is generated
        let no_more_moderator_after_ack =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;
        assert!(
            no_more_moderator_after_ack.is_err(),
            "Expected no more messages on moderator slim channel after ack"
        );

        let no_more_participant_after_ack =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant_after_ack.is_err(),
            "Expected no more messages on participant slim channel after ack"
        );

        // create a leave request and send to moderator on message (direction south)
        let leave_request = Message::builder()
            .source(moderator_name.clone())
            .destination(participant_name.clone())
            .identity("")
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::LeaveRequest)
            .session_id(session_id)
            .message_id(rand::random::<u32>())
            .payload(CommandPayload::builder().leave_request(None).as_content())
            .build_publish()
            .unwrap();

        moderator
            .on_message_from_app(leave_request)
            .await
            .expect("error sending leave request");

        // check that the request is received by slim on the moderator
        let received_leave_request = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for leave request on moderator slim channel")
            .expect("channel closed")
            .expect("error in leave request");

        assert_eq!(
            received_leave_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::LeaveRequest
        );
        assert_eq!(received_leave_request.get_dst(), participant_name_id);

        // send the request to the participant (direction north)
        let mut leave_request_to_participant = received_leave_request.clone();
        leave_request_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message_from_slim(leave_request_to_participant)
            .await
            .expect("error processing leave request on participant");

        // get the leave reply on the participant slim
        let leave_reply = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for leave reply on participant slim channel")
            .expect("channel closed")
            .expect("error in leave reply");

        assert_eq!(
            leave_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::LeaveReply
        );
        assert_eq!(leave_reply.get_dst(), moderator_name);

        // get the delete route on the participant slim
        let delete_route = timeout(Duration::from_millis(600), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for delete route on participant slim channel")
            .expect("channel closed")
            .expect("error in delete route");

        assert!(
            delete_route.is_unsubscribe(),
            "delete route should be an unsubscribe message"
        );
        assert_eq!(delete_route.get_dst(), moderator_name);

        // send the leave reply to the moderator on message (direction north)
        let mut leave_reply_to_moderator = leave_reply.clone();
        leave_reply_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message_from_slim(leave_reply_to_moderator)
            .await
            .expect("error processing leave reply on moderator");

        // expect a remove route for the participant name
        let delete_route = timeout(Duration::from_millis(600), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for delete route on participant slim channel")
            .expect("channel closed")
            .expect("error in delete route");

        assert!(
            delete_route.is_unsubscribe(),
            "delete route should be an unsubscribe message"
        );
        assert_eq!(delete_route.get_dst(), participant_name_id);

        // check that no other messages are generated by the moderator
        let no_more_moderator_final =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;

        assert!(
            no_more_moderator_final.is_err(),
            "Expected no more messages on moderator slim channel after leave"
        );

        let no_more_participant_final =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant_final.is_err(),
            "Expected no more messages on participant slim channel after leave"
        );
    }

    // ============================================================================
    // Draining Tests
    #[traced_test]
    #[tokio::test]
    async fn test_internal_draining_via_processing_state_switch() {
        use super::*;
        use tokio::sync::mpsc;
        use tracing::debug;

        // Custom handler that flips processing_state to Draining after first normal message
        struct InternalDrainHandler {
            state: ProcessingState,
            messages: Vec<SessionMessage>,
            needs_drain: Arc<AtomicBool>,
        }

        impl InternalDrainHandler {
            fn new(needs_drain: Arc<AtomicBool>) -> Self {
                Self {
                    state: ProcessingState::Active,
                    messages: vec![],
                    needs_drain,
                }
            }
        }

        #[async_trait::async_trait]
        impl MessageHandler for InternalDrainHandler {
            async fn init(&mut self) -> Result<(), SessionError> {
                Ok(())
            }

            async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
                debug!(?self.state, "internal-drain-handler received message");
                self.messages.push(message);

                // when we receive 2 messages, transition to draining state
                if self.messages.len() == 2 {
                    debug!("internal-drain-handler transitioning to draining");
                    self.state = ProcessingState::Draining;
                }

                Ok(())
            }

            fn needs_drain(&self) -> bool {
                self.needs_drain.load(std::sync::atomic::Ordering::SeqCst)
            }

            fn processing_state(&self) -> ProcessingState {
                self.state
            }

            async fn on_shutdown(&mut self) -> Result<(), SessionError> {
                debug!("shutdown called on handler");
                Ok(())
            }
        }

        // Build minimal SessionSettings
        let (tx_slim, _rx_slim) = mpsc::channel(8);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, rx_session) = mpsc::channel(32);
        let (tx_session_layer, _rx_session_layer) = mpsc::channel(8);

        let settings = SessionSettings {
            id: 999,
            source: Name::from_strings(["org", "ns", "source"]).with_id(1),
            destination: Name::from_strings(["org", "ns", "dest"]).with_id(2),
            config: SessionConfig {
                session_type: ProtoSessionType::PointToPoint,
                max_retries: Some(3),
                interval: Some(Duration::from_millis(150)),
                mls_enabled: false,
                initiator: true,
                metadata: HashMap::new(),
            },
            tx: SessionTransmitter::new(tx_slim, tx_app),
            tx_session: tx_session.clone(),
            tx_to_session_layer: tx_session_layer,
            identity_provider: SharedSecret::new("src", SHARED_SECRET),
            identity_verifier: SharedSecret::new("src", SHARED_SECRET),
            storage_path: std::path::PathBuf::from("/tmp/internal_draining_test"),
            graceful_shutdown_timeout: Some(Duration::from_secs(10)),
        };

        let needs_drain = Arc::new(AtomicBool::new(true));
        let handler = InternalDrainHandler::new(needs_drain.clone());
        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        // Spawn processing loop without unnecessary cloning
        let processing_handle = tokio::spawn(async move {
            SessionController::processing_loop(
                handler,
                rx_session,
                cancellation_token_clone,
                settings,
            )
            .await
        });

        // Send first regular message
        tx_session
            .send(create_test_message(1, b"first".to_vec()))
            .await
            .expect("failed to send first message");

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(logs_contain("internal-drain-handler received message"));

        // Send second message; this causes internal handler move to draining (active -> draining)
        tx_session
            .send(create_test_message(2, b"second".to_vec()))
            .await
            .expect("failed to send second message");

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(logs_contain("internal-drain-handler received message"));

        assert!(logs_contain(
            "internal-drain-handler transitioning to draining"
        ));

        // Send a third message that should not be processed, as draining is active
        tx_session
            .send(create_test_message(3, b"third".to_vec()))
            .await
            .expect("failed to send third message");

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(logs_contain(
            "session is draining, rejecting new messages from application"
        ));

        // set needs drain to false to allow shutdown to complete
        needs_drain.store(false, std::sync::atomic::Ordering::SeqCst);

        // trigger cancellation to exit processing loop
        cancellation_token.cancel();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send a session message to trigger the shutdown process
        tx_session
            .send(SessionMessage::StartDrain {
                grace_period: std::time::Duration::from_millis(100),
            })
            .await
            .expect("failed to send timeout message");

        // Wait for processing loop to complete
        processing_handle.await.expect("processing loop panicked");
    }
    // ============================================================================

    /// Mock handler that tracks draining behavior
    struct DrainableHandler {
        messages_received: Arc<tokio::sync::Mutex<Vec<SessionMessage>>>,
        needs_drain: Arc<AtomicBool>,
        shutdown_called: Arc<tokio::sync::Mutex<bool>>,
        drain_delay: Option<Duration>,
    }

    impl DrainableHandler {
        fn new() -> Self {
            Self {
                messages_received: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                needs_drain: Arc::new(AtomicBool::new(false)),
                shutdown_called: Arc::new(tokio::sync::Mutex::new(false)),
                drain_delay: None,
            }
        }

        fn with_needs_drain(self, needs_drain: bool) -> Self {
            self.needs_drain
                .store(needs_drain, std::sync::atomic::Ordering::SeqCst);
            self
        }

        #[allow(dead_code)]
        fn with_drain_delay(mut self, delay: Duration) -> Self {
            self.drain_delay = Some(delay);
            self
        }

        #[allow(dead_code)]
        async fn get_messages_count(&self) -> usize {
            self.messages_received.lock().await.len()
        }

        #[allow(dead_code)]
        async fn was_shutdown_called(&self) -> bool {
            *self.shutdown_called.lock().await
        }
    }

    #[async_trait::async_trait]
    impl MessageHandler for DrainableHandler {
        async fn init(&mut self) -> Result<(), SessionError> {
            Ok(())
        }

        async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
            self.messages_received.lock().await.push(message);
            Ok(())
        }

        fn needs_drain(&self) -> bool {
            self.needs_drain.load(std::sync::atomic::Ordering::SeqCst)
        }

        async fn on_shutdown(&mut self) -> Result<(), SessionError> {
            if let Some(delay) = self.drain_delay {
                tokio::time::sleep(delay).await;
            }
            *self.shutdown_called.lock().await = true;
            Ok(())
        }
    }

    /// Helper to create test SessionSettings
    fn create_test_settings(
        graceful_shutdown_timeout: Option<Duration>,
    ) -> SessionSettings<SharedSecret, SharedSecret> {
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(10);
        let (tx_app, _rx_app) = tokio::sync::mpsc::unbounded_channel();
        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(10);
        let (tx_session_layer, _rx_session_layer) = tokio::sync::mpsc::channel(10);

        SessionSettings {
            id: 1,
            source: Name::from_strings(["org", "ns", "test"]).with_id(1),
            destination: Name::from_strings(["org", "ns", "test"]).with_id(2),
            config: SessionConfig {
                session_type: ProtoSessionType::PointToPoint,
                max_retries: Some(5),
                interval: Some(Duration::from_millis(200)),
                mls_enabled: false,
                initiator: true,
                metadata: HashMap::new(),
            },
            tx: SessionTransmitter::new(tx_slim, tx_app),
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider: SharedSecret::new("test", SHARED_SECRET),
            identity_verifier: SharedSecret::new("test", SHARED_SECRET),
            storage_path: std::path::PathBuf::from("/tmp/test_draining"),
            graceful_shutdown_timeout,
        }
    }

    /// Helper to create a test message
    fn create_test_message(message_id: u32, payload: Vec<u8>) -> SessionMessage {
        SessionMessage::OnMessage {
            message: Message::builder()
                .source(Name::from_strings(["org", "ns", "test"]).with_id(1))
                .destination(Name::from_strings(["org", "ns", "test"]).with_id(2))
                .identity("")
                .forward_to(1)
                .session_type(ProtoSessionType::PointToPoint)
                .session_message_type(ProtoSessionMessageType::Msg)
                .session_id(1)
                .message_id(message_id)
                .application_payload("test", payload)
                .build_publish()
                .unwrap(),
            direction: MessageDirection::South,
            ack_tx: None,
        }
    }

    async fn count_on_messages(messages: &Arc<tokio::sync::Mutex<Vec<SessionMessage>>>) -> usize {
        let messages = messages.lock().await;
        messages
            .iter()
            .filter(|msg| matches!(msg, SessionMessage::OnMessage { .. }))
            .count()
    }

    /// Helper to spawn a processing loop and return the task handle
    fn spawn_processing_loop(
        handler: DrainableHandler,
        rx: tokio::sync::mpsc::Receiver<SessionMessage>,
        cancellation_token: CancellationToken,
        settings: SessionSettings<SharedSecret, SharedSecret>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            SessionController::processing_loop(handler, rx, cancellation_token, settings).await;
        })
    }

    #[tokio::test]
    async fn test_draining_processes_queued_messages() {
        let handler = DrainableHandler::new();
        let messages_received = handler.messages_received.clone();
        let shutdown_called = handler.shutdown_called.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_secs(2)));
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Send multiple messages before cancellation
        tx.send(create_test_message(1, vec![1, 2, 3]))
            .await
            .unwrap();
        tx.send(create_test_message(2, vec![4, 5, 6]))
            .await
            .unwrap();
        tx.send(create_test_message(3, vec![7, 8, 9]))
            .await
            .unwrap();

        // Give some time for messages to be queued
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger cancellation
        token_clone.cancel();

        // Close the channel to signal no more messages
        drop(tx);

        // Wait for processing to complete
        timeout(Duration::from_secs(3), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        // Verify all messages were processed
        let processed_messages = count_on_messages(&messages_received).await;
        assert_eq!(
            processed_messages, 3,
            "All queued messages should be processed during draining"
        );
        assert!(
            *shutdown_called.lock().await,
            "Shutdown should have been called"
        );
    }

    #[tokio::test]
    async fn test_draining_with_needs_drain_true() {
        let handler = DrainableHandler::new().with_needs_drain(true);
        let messages_received = handler.messages_received.clone();
        let shutdown_called = handler.shutdown_called.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_secs(2)));
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Send a message
        tx.send(create_test_message(1, vec![1, 2, 3]))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger cancellation and close channel
        token_clone.cancel();
        drop(tx);

        // Wait for processing to complete (should wait for drain timeout)
        timeout(Duration::from_secs(3), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        // Verify message was processed and shutdown was called
        let processed_messages = count_on_messages(&messages_received).await;
        assert_eq!(processed_messages, 1, "Message should be processed");
        assert!(
            *shutdown_called.lock().await,
            "Shutdown should have been called after draining"
        );
    }

    #[tokio::test]
    async fn test_draining_with_needs_drain_false() {
        let handler = DrainableHandler::new().with_needs_drain(false);
        let messages_received = handler.messages_received.clone();
        let shutdown_called = handler.shutdown_called.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_secs(2)));

        let start_time = tokio::time::Instant::now();
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Send a message
        tx.send(create_test_message(1, vec![1, 2, 3]))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger cancellation and close channel
        token_clone.cancel();
        drop(tx);

        // Wait for processing to complete (should exit quickly)
        timeout(Duration::from_secs(1), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        let elapsed = start_time.elapsed();

        // Verify message was processed and shutdown was called quickly
        let processed_messages = count_on_messages(&messages_received).await;
        assert_eq!(processed_messages, 1, "Message should be processed");
        assert!(
            *shutdown_called.lock().await,
            "Shutdown should have been called"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "Should exit quickly when no draining needed"
        );
    }

    #[tokio::test]
    async fn test_draining_timeout_enforced() {
        // Test that the timeout fires when draining takes too long with needs_drain=true
        let handler = DrainableHandler::new().with_needs_drain(true);

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_millis(500)));
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Give the processing loop a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let start_time = tokio::time::Instant::now();

        // Trigger cancellation - this starts draining
        token_clone.cancel();

        // Keep sending messages to prevent channel from closing
        // This simulates a scenario where messages keep arriving during drain period
        let send_task = tokio::spawn(async move {
            for i in 0..10 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if tx
                    .send(create_test_message(i, vec![i as u8]))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        // Wait for processing to complete - should timeout after 500ms
        timeout(Duration::from_secs(2), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        let elapsed = start_time.elapsed();

        // Verify timeout was enforced (should be around 500ms)
        assert!(
            elapsed >= Duration::from_millis(400),
            "Should wait at least close to the timeout period"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "Should respect the timeout and exit, not wait forever"
        );

        // Clean up the send task
        send_task.abort();
    }

    #[tokio::test]
    async fn test_draining_no_messages_in_queue() {
        let handler = DrainableHandler::new().with_needs_drain(true);
        let messages_received = handler.messages_received.clone();
        let shutdown_called = handler.shutdown_called.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_secs(1)));
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Trigger cancellation immediately without sending messages
        token_clone.cancel();
        drop(tx);

        // Wait for processing to complete
        timeout(Duration::from_secs(2), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        // Verify no messages were processed but shutdown was called
        let processed_messages = count_on_messages(&messages_received).await;
        assert_eq!(processed_messages, 0, "No messages should be processed");
        assert!(
            *shutdown_called.lock().await,
            "Shutdown should still be called"
        );
    }

    #[tokio::test]
    async fn test_draining_messages_after_cancellation_processed() {
        let handler = DrainableHandler::new();
        let messages_received = handler.messages_received.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        let settings = create_test_settings(Some(Duration::from_secs(2)));

        // Send messages before cancellation
        tx.send(create_test_message(1, vec![1, 2, 3]))
            .await
            .unwrap();
        tx.send(create_test_message(2, vec![4, 5, 6]))
            .await
            .unwrap();

        // Spawn the processing loop after messages are queued
        let processing_task = spawn_processing_loop(handler, rx, cancellation_token, settings);

        // Give a moment for processing to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Trigger cancellation while messages are in queue
        token_clone.cancel();

        // Close channel
        drop(tx);

        // Wait for processing to complete
        timeout(Duration::from_secs(3), processing_task)
            .await
            .expect("timeout waiting for processing loop")
            .expect("processing loop panicked");

        // Verify messages in queue when cancellation happened were still processed
        let processed_messages = count_on_messages(&messages_received).await;
        assert_eq!(
            processed_messages, 2,
            "Messages in queue during cancellation should be processed"
        );
    }
}
