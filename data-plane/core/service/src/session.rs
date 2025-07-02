// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use parking_lot::{RwLock, RwLockReadGuard};
use slim_auth::traits::{TokenProvider, Verifier};
use tonic::Status;

use crate::errors::SessionError;
use crate::fire_and_forget::{FireAndForget, FireAndForgetConfiguration};
use crate::request_response::{RequestResponse, RequestResponseConfiguration};
use crate::streaming::{Streaming, StreamingConfiguration};
use slim_datapath::api::proto::pubsub::v1::{Message, SessionHeaderType};
use slim_datapath::messages::encoder::Agent;

/// Session ID
pub type Id = u32;

/// Reserved session id
pub const SESSION_RANGE: std::ops::Range<u32> = 0..(u32::MAX - 1000);
pub const SESSION_UNSPECIFIED: u32 = u32::MAX;

/// The session
pub(crate) enum Session<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Fire and forget session
    FireAndForget(FireAndForget<P, V>),
    /// Request response session
    RequestResponse(RequestResponse<P, V>),
    /// Streaming session
    Streaming(Streaming<P, V>),
}

/// Message wrapper
#[derive(Clone, PartialEq, Debug)]
pub struct SessionMessage {
    /// The message to be sent
    pub message: Message,
    /// The optional session info
    pub info: Info,
}

impl SessionMessage {
    /// Create a new session message
    pub fn new(message: Message, info: Info) -> Self {
        SessionMessage { message, info }
    }
}

impl From<(Message, Info)> for SessionMessage {
    fn from(tuple: (Message, Info)) -> Self {
        SessionMessage {
            message: tuple.0,
            info: tuple.1,
        }
    }
}

impl From<Message> for SessionMessage {
    fn from(message: Message) -> Self {
        let info = Info::from(&message);
        SessionMessage { message, info }
    }
}

impl From<SessionMessage> for Message {
    fn from(session_message: SessionMessage) -> Self {
        session_message.message
    }
}

/// Channel used in the path service -> app
pub type AppChannelSender = tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::Receiver<Result<SessionMessage, SessionError>>;
/// Channel used in the path service -> slim
pub type SlimChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;
/// Channel used in the path slim -> service
pub type SlimChannelReceiver = tokio::sync::mpsc::Receiver<Result<Message, Status>>;

/// Session Info
#[derive(Clone, PartialEq, Debug)]
pub struct Info {
    /// The id of the session
    pub id: Id,
    /// The message nonce used to identify the message
    pub message_id: Option<u32>,
    /// The Message Type
    pub session_header_type: SessionHeaderType,
    /// The identifier of the agent that sent the message
    pub message_source: Option<Agent>,
    /// The input connection id
    pub input_connection: Option<u64>,
}

impl Info {
    /// Create a new session info
    pub fn new(id: Id) -> Self {
        Info {
            id,
            message_id: None,
            session_header_type: SessionHeaderType::Unspecified,
            message_source: None,
            input_connection: None,
        }
    }

    pub fn set_message_id(&mut self, message_id: u32) {
        self.message_id = Some(message_id);
    }

    pub fn set_session_header_type(&mut self, session_header_type: SessionHeaderType) {
        self.session_header_type = session_header_type;
    }

    pub fn set_message_source(&mut self, message_source: Agent) {
        self.message_source = Some(message_source);
    }

    pub fn set_input_connection(&mut self, input_connection: u64) {
        self.input_connection = Some(input_connection);
    }

    pub fn get_message_id(&self) -> Option<u32> {
        self.message_id
    }

    pub fn get_session_header_type(&self) -> SessionHeaderType {
        self.session_header_type
    }

    pub fn get_message_source(&self) -> Option<Agent> {
        self.message_source.clone()
    }

    pub fn get_input_connection(&self) -> Option<u64> {
        self.input_connection
    }
}

impl From<&Message> for Info {
    fn from(message: &Message) -> Self {
        let session_header = message.get_session_header();
        let slim_header = message.get_slim_header();

        let id = session_header.session_id;
        let message_id = session_header.message_id;
        let message_source = message.get_source();
        let input_connection = slim_header.incoming_conn;
        let session_header_type = session_header.header_type;

        Info {
            id,
            message_id: Some(message_id),
            session_header_type: SessionHeaderType::try_from(session_header_type)
                .unwrap_or(SessionHeaderType::Unspecified),
            message_source: Some(message_source),
            input_connection,
        }
    }
}

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Active,
    Inactive,
}

/// The type of a session
#[derive(Clone, PartialEq, Debug)]
pub enum SessionDirection {
    #[allow(dead_code)]
    Sender,
    #[allow(dead_code)]
    Receiver,
    Bidirectional,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum MessageDirection {
    North,
    South,
}

/// The session type
#[derive(Clone, PartialEq, Debug)]
pub enum SessionType {
    FireAndForget,
    RequestResponse,
    Streaming,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::FireAndForget => write!(f, "FireAndForget"),
            SessionType::RequestResponse => write!(f, "RequestResponse"),
            SessionType::Streaming => write!(f, "Streaming"),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum SessionConfig {
    FireAndForget(FireAndForgetConfiguration),
    RequestResponse(RequestResponseConfiguration),
    Streaming(StreamingConfiguration),
}

pub trait SessionConfigTrait {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError>;
}

impl std::fmt::Display for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionConfig::FireAndForget(ff) => write!(f, "{}", ff),
            SessionConfig::RequestResponse(rr) => write!(f, "{}", rr),
            SessionConfig::Streaming(s) => write!(f, "{}", s),
        }
    }
}

pub(crate) trait CommonSession<P, V>: Interceptor
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Session ID
    #[allow(dead_code)]
    fn id(&self) -> Id;

    // get the session state
    #[allow(dead_code)]
    fn state(&self) -> &State;

    /// Get the token provider
    #[allow(dead_code)]
    fn identity_provider(&self) -> P;

    /// Get the verifier
    #[allow(dead_code)]
    fn identity_verifier(&self) -> V;

    /// Get the source name
    fn source(&self) -> &Agent;

    // get the session config
    fn session_config(&self) -> SessionConfig;

    // set the session config
    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError>;

    /// Intercept message from app
    fn on_message_from_app_interceptors(&self, msg: &mut Message) -> Result<(), SessionError>;

    /// Intercept message from slim
    fn on_message_from_slim_interceptors(&self, msg: &mut Message) -> Result<(), SessionError>;
}

pub(crate) trait Interceptor {
    fn add_interceptor(&self, interceptor: Box<dyn SessionInterceptor + Send + Sync + 'static>);
}

pub trait SessionInterceptor {
    // interceptor to be executed when a message is received from the app
    fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError>;
    // interceptor to be executed when a message is received from slim
    fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError>;
}

#[async_trait]
pub(crate) trait MessageHandler {
    // publish a message as part of the session
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError>;
}

/// Common session data
pub(crate) struct Common<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Session ID - unique identifier for the session
    #[allow(dead_code)]
    id: Id,

    /// Session state
    #[allow(dead_code)]
    state: State,

    /// Token provider for authentication
    #[allow(dead_code)]
    identity_provider: P,

    /// Verifier for authentication
    #[allow(dead_code)]
    identity_verifier: V,

    /// Session type
    session_config: RwLock<SessionConfig>,

    /// Session direction
    #[allow(dead_code)]
    session_direction: SessionDirection,

    /// Source agent
    source: Agent,

    /// Sender for messages to slim
    tx_slim: SlimChannelSender,

    /// Sender for messages to app
    tx_app: AppChannelSender,

    // Interceptors to be called on message reception/send
    interceptors: RwLock<Vec<Box<dyn SessionInterceptor + Send + Sync>>>,
}

#[async_trait]
impl<P, V> MessageHandler for Session<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match self {
            Session::FireAndForget(session) => session.on_message(message, direction).await,
            Session::RequestResponse(session) => session.on_message(message, direction).await,
            Session::Streaming(session) => session.on_message(message, direction).await,
        }
    }
}

impl<P, V> Interceptor for Session<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn add_interceptor(&self, interceptor: Box<dyn SessionInterceptor + Send + Sync + 'static>) {
        match self {
            Session::FireAndForget(session) => session.add_interceptor(interceptor),
            Session::RequestResponse(session) => session.add_interceptor(interceptor),
            Session::Streaming(session) => session.add_interceptor(interceptor),
        }
    }
}

impl<P, V> CommonSession<P, V> for Session<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        match self {
            Session::FireAndForget(session) => session.id(),
            Session::RequestResponse(session) => session.id(),
            Session::Streaming(session) => session.id(),
        }
    }

    fn state(&self) -> &State {
        match self {
            Session::FireAndForget(session) => session.state(),
            Session::RequestResponse(session) => session.state(),
            Session::Streaming(session) => session.state(),
        }
    }

    fn identity_provider(&self) -> P {
        match self {
            Session::FireAndForget(session) => session.identity_provider(),
            Session::RequestResponse(session) => session.identity_provider(),
            Session::Streaming(session) => session.identity_provider(),
        }
    }

    fn identity_verifier(&self) -> V {
        match self {
            Session::FireAndForget(session) => session.identity_verifier(),
            Session::RequestResponse(session) => session.identity_verifier(),
            Session::Streaming(session) => session.identity_verifier(),
        }
    }

    fn source(&self) -> &Agent {
        match self {
            Session::FireAndForget(session) => session.source(),
            Session::RequestResponse(session) => session.source(),
            Session::Streaming(session) => session.source(),
        }
    }

    fn session_config(&self) -> SessionConfig {
        match self {
            Session::FireAndForget(session) => session.session_config(),
            Session::RequestResponse(session) => session.session_config(),
            Session::Streaming(session) => session.session_config(),
        }
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match self {
            Session::FireAndForget(session) => session.set_session_config(session_config),
            Session::RequestResponse(session) => session.set_session_config(session_config),
            Session::Streaming(session) => session.set_session_config(session_config),
        }
    }

    fn on_message_from_app_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        match self {
            Session::FireAndForget(session) => session.on_message_from_app_interceptors(msg),
            Session::RequestResponse(session) => session.on_message_from_app_interceptors(msg),
            Session::Streaming(session) => session.on_message_from_app_interceptors(msg),
        }
    }

    fn on_message_from_slim_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        match self {
            Session::FireAndForget(session) => session.on_message_from_slim_interceptors(msg),
            Session::RequestResponse(session) => session.on_message_from_slim_interceptors(msg),
            Session::Streaming(session) => session.on_message_from_slim_interceptors(msg),
        }
    }
}

impl<P, V> CommonSession<P, V> for Common<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        self.id
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn source(&self) -> &Agent {
        &self.source
    }

    fn session_config(&self) -> SessionConfig {
        self.session_config.read().clone()
    }

    fn identity_provider(&self) -> P {
        self.identity_provider.clone()
    }

    fn identity_verifier(&self) -> V {
        self.identity_verifier.clone()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        let mut conf = self.session_config.write();

        match *conf {
            SessionConfig::FireAndForget(ref mut config) => {
                config.replace(session_config)?;
            }
            SessionConfig::RequestResponse(ref mut config) => {
                config.replace(session_config)?;
            }
            SessionConfig::Streaming(ref mut config) => {
                config.replace(session_config)?;
            }
        }
        Ok(())
    }

    fn on_message_from_app_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = RwLockReadGuard::map(self.interceptors.read(), |x| x);
        for i in interceptors.iter() {
            i.on_msg_from_app(msg)?;
        }
        Ok(())
    }

    fn on_message_from_slim_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = RwLockReadGuard::map(self.interceptors.read(), |x| x);
        for i in interceptors.iter() {
            i.on_msg_from_slim(msg)?;
        }
        Ok(())
    }
}

impl<P, V> Interceptor for Common<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn add_interceptor(&self, interceptor: Box<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.write().push(interceptor);
    }
}

impl<P, V> Common<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        session_config: SessionConfig,
        source: Agent,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
        identity_provider: P,
        verifier: V,
    ) -> Self {
        Self {
            id,
            state: State::Active,
            identity_provider,
            identity_verifier: verifier,
            session_direction,
            session_config: RwLock::new(session_config),
            source,
            tx_slim,
            tx_app,
            interceptors: RwLock::new(vec![]),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

    pub(crate) fn tx_slim_ref(&self) -> &SlimChannelSender {
        &self.tx_slim
    }

    #[allow(dead_code)]
    pub(crate) fn tx_app(&self) -> AppChannelSender {
        self.tx_app.clone()
    }

    pub(crate) fn tx_app_ref(&self) -> &AppChannelSender {
        &self.tx_app
    }
}
