// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use bincode::{Decode, Encode};
use parking_lot::Mutex;
use tracing::{debug, error, info, trace};

use crate::{
    errors::SessionError,
    interceptor_mls::{METADATA_MLS_ENABLED, METADATA_MLS_INIT_COMMIT_ID},
    session::{Id, SessionTransmitter},
};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        SessionHeader, SlimHeader,
        proto::pubsub::v1::{Message, SessionHeaderType},
    },
    messages::{Agent, AgentType, utils::SlimHeaderFlags},
};
use slim_mls::mls::Mls;

struct RequestTimerObserver<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// message to send in case of timeout
    message: Message,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

#[async_trait]
impl<T> crate::timer::TimerObserver for RequestTimerObserver<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for request {}", timeouts, timer_id);

        if self
            .tx
            .send_to_slim(Ok(self.message.clone()))
            .await
            .is_err()
        {
            error!("error sending invite message");
        }
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        error!("unable to send message {:?}, stop retrying", self.message);
        self.tx
            .send_to_app(Err(SessionError::Processing(
                "timer failed on channel endpoint. Stop sending messages".to_string(),
            )))
            .await
            .expect("error notifying app");
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!("timer for rtx {} cancelled", timer_id);
        // nothing to do
    }
}

trait OnMessageReceived {
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError>;
}

#[derive(Debug)]
pub(crate) enum ChannelEndpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    ChannelParticipant(ChannelParticipant<P, V, T>),
    ChannelModerator(ChannelModerator<P, V, T>),
}

impl<P, V, T> ChannelEndpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        match self {
            ChannelEndpoint::ChannelParticipant(cp) => cp.on_message(msg).await,
            ChannelEndpoint::ChannelModerator(cm) => cm.on_message(msg).await,
        }
    }
}

#[derive(Debug)]
pub(crate) struct MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state for the channel of this endpoint
    /// the mls state should be created and initiated in the app
    /// so that it can be shared with the channel and the interceptors
    mls: Arc<Mutex<Mls<P, V>>>,

    /// used only if Some(mls)
    group: Vec<u8>,

    /// last commit id
    last_commit_id: u32,

    /// map of the participants with package keys
    /// this is used only by the moderator to remove
    /// participants from the channel
    participants: HashMap<Agent, Vec<u8>>,
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(mls: Arc<Mutex<Mls<P, V>>>) -> Result<Self, SessionError> {
        mls.lock()
            .initialize()
            .map_err(|e| SessionError::MLSInit(e.to_string()))?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_commit_id: 0,
            participants: HashMap::new(),
        })
    }

    async fn init_moderator(&mut self) -> Result<(), SessionError> {
        self.mls
            .lock()
            .create_group()
            .map(|_| ())
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    async fn generate_key_package(&mut self) -> Result<Vec<u8>, SessionError> {
        self.mls
            .lock()
            .generate_key_package()
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    async fn process_welcome_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.last_commit_id != 0 {
            debug!("welcome message already received, drop");
            // we already got a welcome message, ignore this one
            return Ok(());
        }

        self.last_commit_id = msg
            .get_metadata(METADATA_MLS_INIT_COMMIT_ID)
            .ok_or(SessionError::WelcomeMessage(
                "received welcome message without commit id, drop it".to_string(),
            ))?
            .parse::<u32>()
            .map_err(|_| {
                SessionError::WelcomeMessage(
                    "received welcome message with invalid commit id, drop it".to_string(),
                )
            })?;

        let welcome = &msg
            .get_payload()
            .ok_or(SessionError::WelcomeMessage(
                "missing payload in MLS welcome, cannot join the group".to_string(),
            ))?
            .blob;

        self.group = self
            .mls
            .lock()
            .process_welcome(welcome)
            .map_err(|e| SessionError::WelcomeMessage(e.to_string()))?;

        Ok(())
    }

    async fn process_commit_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        // the first message to be received should be a welcome message
        // this message will init the last_commit_id. so if last_commit_id = 0
        // drop the commits
        if self.last_commit_id == 0 {
            error!("welcome message not received yet, drop commit");
            return Err(SessionError::CommitMessage(
                "welcome message not received yet, drop commit".to_string(),
            ));
        }

        // the only valid commit that we can accepet it the commit with id
        // last_commit_id + 1. We can safely drop all the others because
        // the moderator will keep sending them if needed
        let msg_id = msg.get_id();
        if msg_id == self.last_commit_id + 1 {
            debug!("received valid commit with id {}", msg_id);
            self.last_commit_id += 1;
        } else {
            error!("unexpected commit id, drop message");
            return Err(SessionError::CommitMessage(
                "unexpected commit id, drop message".to_string(),
            ));
        }

        let commit = &msg
            .get_payload()
            .ok_or(SessionError::CommitMessage(
                "missing payload in MLS commit, cannot process the commit".to_string(),
            ))?
            .blob;

        self.mls
            .lock()
            .process_commit(commit)
            .map_err(|e| SessionError::CommitMessage(e.to_string()))
    }

    async fn add_participant(
        &mut self,
        msg: &Message,
    ) -> Result<(Vec<u8>, Vec<u8>), SessionError> {
        let payload = &msg
            .get_payload()
            .ok_or(SessionError::AddParticipant(
                "key package is missing. the end point cannot be added to the channel".to_string(),
            ))?
            .blob;

        match self.mls.lock().add_member(payload) {
            Ok(ret) => {
                // add participant to the list
                self.participants
                    .insert(msg.get_source(), ret.member_identity);
                Ok((ret.commit_message, ret.welcome_message))
            }
            Err(e) => {
                error!("error adding new endpoint {}", e.to_string());
                Err(SessionError::AddParticipant(e.to_string()))
            }
        }
    }

    async fn remove_participant(&mut self, msg: &Message) {
        println!("----remove participant");
        let name = msg.get_name_as_agent();
        let id = match self.participants.get(&name) {
            Some(id) => id,
            None => {
                error!("the name does not exists in the group");
                return;
            }
        };

        println!("----call mls");
        let _ = self.mls.lock().remove_member(id);
    }
}

#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct JoinMessagePayload {
    channel_name: AgentType,
    moderator_name: Agent,
}

impl JoinMessagePayload {
    fn new(channel_name: AgentType, moderator_name: Agent) -> Self {
        JoinMessagePayload {
            channel_name,
            moderator_name,
        }
    }
}

#[derive(Debug)]
struct Endpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// endpoint name
    name: Agent,

    /// channel name
    channel_name: AgentType,

    /// channel id, used to exchange messages with a single endpoint
    #[allow(dead_code)]
    channel_id: Option<u64>,

    /// id of the current session
    session_id: Id,

    /// connection id to the next hop SLIM
    conn: Option<u64>,

    /// true is the endpoint is already subscribed to the channel
    subscribed: bool,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

impl<P, V, T> Endpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        mls_state: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        Endpoint {
            name,
            channel_name,
            channel_id,
            session_id,
            conn: None,
            subscribed: false,
            mls_state,
            tx,
        }
    }

    fn create_channel_message(
        &self,
        destination: &AgentType,
        destination_id: Option<u64>,
        broadcast: bool,
        request_type: SessionHeaderType,
        message_id: u32,
        payload: Vec<u8>,
    ) -> Message {
        let flags = if broadcast {
            Some(SlimHeaderFlags::new(10, None, None, None, None))
        } else {
            None
        };

        let slim_header = Some(SlimHeader::new(
            &self.name,
            destination,
            destination_id,
            flags,
        ));

        let session_header = Some(SessionHeader::new(
            request_type.into(),
            self.session_id,
            message_id,
        ));

        Message::new_publish_with_headers(slim_header, session_header, "", payload)
    }

    async fn join(&mut self) -> Result<(), SessionError> {
        // subscribe only once to the channel
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        // subscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let sub = Message::new_subscribe(&self.name, &self.channel_name, None, header);

        self.send(sub).await?;

        // set rout for the channel
        self.set_route(&self.channel_name, None).await
    }

    async fn set_route(
        &self,
        route_name: &AgentType,
        route_id: Option<u64>,
    ) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_subscribe(
            &self.name,
            route_name,
            route_id,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn delete_route(&self, route_name: &AgentType, route_id: Option<u64>) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_unsubscribe(
            &self.name,
            route_name,
            route_id,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn leave(&self) -> Result<(), SessionError> {
        // unsubscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let unsub = Message::new_unsubscribe(&self.name, &self.channel_name, None, header);

        self.send(unsub).await?;

        // remove route for the channel
        self.delete_route(&self.channel_name, None).await
    }

    async fn send(&self, msg: Message) -> Result<(), SessionError> {
        self.tx.send_to_slim(Ok(msg)).await
    }
}

#[derive(Debug)]
pub struct ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    moderator_name: Option<Agent>,
    endpoint: Endpoint<P, V, T>,
}

impl<P, V, T> ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, channel_id,  session_id, mls, tx);
        ChannelParticipant {
            moderator_name: None,
            endpoint,
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the payload
        let names = msg
            .get_payload()
            .map_or_else(
                || {
                    error!("missing payload in a Join Channel request, ignore the message");
                    Err(SessionError::Processing(
                        "missing payload in a Join Channel request".to_string(),
                    ))
                },
                |content| -> Result<(JoinMessagePayload, usize), SessionError> {
                    bincode::decode_from_slice(&content.blob, bincode::config::standard())
                        .map_err(|e| SessionError::JoinChannelPayload(e.to_string()))
                },
            )?
            .0;

        // set local state according to the info in the message
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.endpoint.session_id = msg.get_session_header().get_session_id();
        self.endpoint.channel_name = names.channel_name;

        // set route in order to be able to send packets to the moderator
        self.endpoint
            .set_route(
                names.moderator_name.agent_type(),
                names.moderator_name.agent_id_option(),
            )
            .await?;

        // set the moderator name after the set route
        self.moderator_name = Some(names.moderator_name);

        // send reply to the moderator
        let src = msg.get_source();
        let payload: Vec<u8> = if msg.contains_metadata(METADATA_MLS_ENABLED) {
            // if mls we need to provide the key package
            self.endpoint
                .mls_state
                .as_mut()
                .ok_or(SessionError::NoMls)?
                .generate_key_package()
                .await?
        } else {
            // without MLS we can set the state for the channel
            // otherwise the endpoint needs to receive a
            // welcome message first
            self.endpoint.join().await?;
            vec![]
        };

        // reply to the request
        let reply = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            SessionHeaderType::ChannelJoinReply,
            msg.get_id(),
            payload,
        );

        self.endpoint.send(reply).await
    }

    async fn on_mls_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        self.endpoint
            .mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_welcome_message(&msg)
            .await?;

        debug!("Welcome message correctly processed, MLS state initialized");

        // set route for the channel name
        self.endpoint.join().await?;

        // send an ack back to the moderator
        let src = msg.get_source();
        let ack = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            SessionHeaderType::ChannelMlsAck,
            msg.get_id(),
            vec![],
        );

        self.endpoint.send(ack).await
    }

    async fn on_mls_commit(&mut self, msg: Message) -> Result<(), SessionError> {
        self.endpoint
            .mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_commit_message(&msg)
            .await?;

        debug!("Commit message correctly processed, MLS state updated");

        // send an ack back to the moderator
        let src = msg.get_source();
        let ack = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            SessionHeaderType::ChannelMlsAck,
            msg.get_id(),
            vec![],
        );

        self.endpoint.send(ack).await
    }
}

impl<P, V, T> OnMessageReceived for ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                error!(
                    "Received discovery request message, this should not happen. drop the message"
                );

                Err(SessionError::Processing(
                    "Received discovery request message, this should not happen".to_string(),
                ))
            }
            SessionHeaderType::ChannelJoinRequest => {
                debug!("Received join request message");
                self.on_join_request(msg).await
            }
            SessionHeaderType::ChannelMlsWelcome => {
                debug!("Received mls welcome message");
                self.on_mls_welcome(msg).await
            }
            SessionHeaderType::ChannelMlsCommit => {
                debug!("Received mls commit message");
                self.on_mls_commit(msg).await
            }
            SessionHeaderType::ChannelLeaveRequest => {
                info!("Received leave request message");


                // reply to the request
                let src = msg.get_source();
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    false,
                    SessionHeaderType::ChannelLeaveReply,
                    msg.get_id(),
                    vec![],
                );

                self.endpoint.send(reply).await?;

                // leave the channel
                self.endpoint.leave().await?;

                match &self.moderator_name {
                    Some(m) => {
                        self.endpoint
                            .delete_route(m.agent_type(), m.agent_id_option())
                            .await?
                    }
                    None => {
                        error!("moderator name is not set, cannot route");
                    }
                };

                Ok(())

            }
            _ => {
                debug!("Received message of type {:?}, drop it", msg_type);

                Err(SessionError::Processing(format!(
                    "Received message of type {:?}, drop it",
                    msg_type
                )))
            }
        }
    }
}

#[derive(Debug)]
pub struct ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    endpoint: Endpoint<P, V, T>,

    /// list of endpoint names in the channel
    // channel_list: HashSet<Agent>,

    /// list of pending requests and related timers
    /// for each timer store also the number of packets that we expect
    /// get back before cancel the timer.
    /// this is needed for broadcast messages.
    pending_requests: HashMap<u32, (crate::timer::Timer, u32)>,

    /// number or maximum retries before give up with a control message
    max_retries: u32,

    /// interval between retries
    retries_interval: Duration,

    /// channel name as payload to add to the invite messages
    invite_payload: Vec<u8>,

    /// mls message id
    mls_msg_id: u32,
}

#[allow(clippy::too_many_arguments)]
impl<P, V, T> ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let p = JoinMessagePayload::new(channel_name.clone(), name.clone());
        let invite_payload: Vec<u8> = bincode::encode_to_vec(p, bincode::config::standard())
            .expect("unable to parse channel name as payload");

        let endpoint = Endpoint::new(name, channel_name, channel_id, session_id, mls, tx);
        ChannelModerator {
            endpoint,
            //channel_list: HashSet::new(),
            pending_requests: HashMap::new(),
            max_retries,
            retries_interval,
            invite_payload,
            mls_msg_id: 0,
        }
    }

    pub async fn join(&mut self) -> Result<(), SessionError> {
        if !self.endpoint.subscribed {
            // join the channel
            self.endpoint.join().await?;

            // create mls group if needed
            if let Some(mls) = self.endpoint.mls_state.as_mut() {
                mls.init_moderator().await?;
            }
        }

        Ok(())
    }

    async fn forward(&mut self, msg: Message) -> Result<(), SessionError> {
        // forward message received from the app and set a timer
        let msg_id = msg.get_id();
        self.endpoint.send(msg.clone()).await?;
        // create a timer for this request
        self.create_timer(msg_id, 1, msg);

        Ok(())
    }

    fn create_timer(&mut self, key: u32, pending_messages: u32, msg: Message) {
        let observer = Arc::new(RequestTimerObserver {
            message: msg,
            tx: self.endpoint.tx.clone(),
        });

        let timer = crate::timer::Timer::new(
            key,
            crate::timer::TimerType::Constant,
            self.retries_interval,
            None,
            Some(self.max_retries),
        );
        timer.start(observer);

        self.pending_requests.insert(key, (timer, pending_messages));
    }

    fn delete_timer(&mut self, key: u32) -> Result<bool, SessionError> {
        let ret = self.pending_requests.get_mut(&key).map_or_else(
            || Err(SessionError::TimerNotFound(key.to_string())),
            |(timer, pending)| {
                if *pending > 0 {
                    *pending -= 1;
                }
                if *pending == 0 {
                    timer.stop();
                    Ok(true)
                } else {
                    Ok(false)
                }
            },
        )?;

        if ret {
            self.pending_requests.remove(&key);
        }

        Ok(ret)
    }

    fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.mls_msg_id += 1;
        self.mls_msg_id
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // set the local state and join the channel
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.join().await?;

        // an endpoint replied to the discovery message
        // send a join message
        let src = msg.get_slim_header().get_source();
        let recv_msg_id = msg.get_id();
        let new_msg_id = rand::random::<u32>();

        // this message cannot be received but it is created here
        let mut join = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            SessionHeaderType::ChannelJoinRequest,
            new_msg_id,
            self.invite_payload.clone(),
        );

        if self.endpoint.mls_state.is_some() {
            join.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());
            debug!("Reply with the join request, MLS is enabled");
        } else {
            debug!("Reply with the join request, MLS is disabled");
        }

        // remove the timer for the discovery message
        self.delete_timer(recv_msg_id)?;

        // add a new one for the join message
        self.create_timer(new_msg_id, 1, join.clone());

        // send the message
        self.endpoint.send(join).await
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let src = msg.get_slim_header().get_source();
        let msg_id = msg.get_id();

        // cancel timer, there only one message pending here
        let ret = self.delete_timer(msg_id)?;
        debug_assert!(ret, "timer for join reply should be removed");

        // send MLS messages if needed
        if self.endpoint.mls_state.is_some() {
            let (commit_payload, welcome_payload) = self
                .endpoint
                .mls_state
                .as_mut()
                .unwrap()
                .add_participant(&msg).await?;

            // send the commit message to the channel
            let commit_id = self.get_next_mls_mgs_id();
            let welcome_id = rand::random::<u32>();

            let commit = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                None,
                true,
                SessionHeaderType::ChannelMlsCommit,
                commit_id,
                commit_payload,
            );
            let mut welcome = self.endpoint.create_channel_message(
                src.agent_type(),
                src.agent_id_option(),
                false,
                SessionHeaderType::ChannelMlsWelcome,
                welcome_id,
                welcome_payload,
            );
            welcome.insert_metadata(
                METADATA_MLS_INIT_COMMIT_ID.to_string(),
                commit_id.to_string(),
            );

            // send welcome message
            debug!("Send MLS Welcome Message to the new participant");
            self.endpoint.send(welcome.clone()).await?;
            self.create_timer(welcome_id, 1, welcome);

            // send commit message if needed
            let len = self.endpoint.mls_state.as_ref().unwrap().participants.len();
            if len > 1 {
                debug!("Send MLS Commit Message to the channel");
                self.endpoint.send(commit.clone()).await?;
                self.create_timer(commit_id, (len - 1).try_into().unwrap(), commit);
            }
        };

        Ok(())
    }

    async fn on_msl_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        let recv_msg_id = msg.get_id();
        let ret = self.delete_timer(recv_msg_id)?;
        debug_assert!(ret, "timer for mls ack should be removed");

        Ok(())
    }
}

impl<P, V, T> OnMessageReceived for ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                debug!("Invite new participant to the channel, send discovery message");
                // discovery message coming from the application
                self.forward(msg).await
            }
            SessionHeaderType::ChannelDiscoveryReply => {
                debug!("Received discovery reply message");
                self.on_discovery_reply(msg).await
            }
            SessionHeaderType::ChannelJoinReply => {
                debug!("Received join reply message");
                self.on_join_reply(msg).await
            }
            SessionHeaderType::ChannelMlsAck => {
                debug!("Received mls ack message");
                self.on_msl_ack(msg).await
            }
            SessionHeaderType::ChannelLeaveRequest => {
                // leave message coming from the application
                info!("Received leave request message");

                self.endpoint
                    .mls_state
                    .as_mut()
                    .unwrap()
                    .remove_participant(&msg)
                    .await;

                self.forward(msg).await

                // TODO
                // if MLS create commit and send it
                //let name = msg.get_name_as_agent();
                /*let id = match self.endpoint.mls_state.as_ref().unwrap().participants.get(&name) {
                    Some(id) => id,
                    None => {
                        error!("the name does not exists in the group");
                        return;
                    }
                };*/
            }
            SessionHeaderType::ChannelLeaveReply => {
                info!("Received leave reply message");
                let src = msg.get_slim_header().get_source();
                let msg_id = msg.get_id();

                // cancel timer
                let ret = self.delete_timer(msg_id)?;
                debug_assert!(ret, "timer for leave reply should be removed");

                // remove from the channel list
                self.endpoint
                    .mls_state
                    .as_mut()
                    .unwrap()
                    .participants
                    .remove(&src);

                // all good
                Ok(())
            }
            _ => Err(SessionError::Processing(format!(
                "received unexpected packet type: {:?}",
                msg_type
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutils::MockTransmitter;

    use super::*;
    use slim_auth::simple::SimpleGroup;
    use tracing_test::traced_test;

    use slim_datapath::messages::AgentType;

    const SESSION_ID: u32 = 10;

    #[tokio::test]
    #[traced_test]
    async fn test_full_join_and_leave() {
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let (moderator_tx, mut moderator_rx) = tokio::sync::mpsc::channel(50);
        let (participant_tx, mut participant_rx) = tokio::sync::mpsc::channel(50);

        let moderator_tx = MockTransmitter {
            tx_app: tx_app.clone(),
            tx_slim: moderator_tx,
        };

        let participant_tx = MockTransmitter {
            tx_app: tx_app.clone(),
            tx_slim: participant_tx,
        };

        let moderator = Agent::from_strings("org", "default", "moderator", 12345);
        let participant = Agent::from_strings("org", "default", "participant", 5120);
        let channel_name = AgentType::from_strings("channel", "channel", "channel");
        let channel_id = Some(1234_u64);
        let conn = 1;

        let moderator_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            moderator.clone(),
            SimpleGroup::new("moderator", "group"),
            SimpleGroup::new("moderator", "group"),
        ))))
        .unwrap();

        let participant_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            participant.clone(),
            SimpleGroup::new("participant", "group"),
            SimpleGroup::new("participant", "group"),
        ))))
        .unwrap();

        let mut cm = ChannelModerator::new(
            moderator.clone(),
            channel_name.clone(),
            channel_id,
            SESSION_ID,
            3,
            Duration::from_millis(100),
            Some(moderator_mls),
            moderator_tx,
        );
        let mut cp = ChannelParticipant::new(
            participant.clone(),
            channel_name.clone(),
            channel_id,
            SESSION_ID,
            Some(participant_mls),
            participant_tx,
        );

        // create a discovery request
        let flags = SlimHeaderFlags::default().with_incoming_conn(conn);

        let slim_header = Some(SlimHeader::new(
            &moderator,
            participant.agent_type(),
            None,
            Some(flags),
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::ChannelDiscoveryRequest.into(),
            SESSION_ID,
            rand::random::<u32>(),
        ));
        let payload: Vec<u8> =
            bincode::encode_to_vec(&moderator, bincode::config::standard()).unwrap();
        let request = Message::new_publish_with_headers(slim_header, session_header, "", payload);

        // receive the request at the session layer
        cm.on_message(request.clone()).await.unwrap();

        // the request is forwarded to slim
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(request, msg);

        // this message is handled by the session layer itself
        // so we can create a reply and send it back to the moderator
        let destination = msg.get_source();
        let msg_id = msg.get_id();
        let session_id = msg.get_session_header().get_session_id();

        let slim_header = Some(SlimHeader::new(
            &participant,
            destination.agent_type(),
            destination.agent_id_option(),
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::ChannelDiscoveryReply.into(),
            session_id,
            msg_id,
        ));

        let mut msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        // message reception on moderator side
        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await.unwrap();

        // the first message is the subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a request to compare with the output of on_message
        let jp = JoinMessagePayload {
            channel_name: channel_name.clone(),
            moderator_name: moderator.clone(),
        };

        let payload: Vec<u8> = bincode::encode_to_vec(&jp, bincode::config::standard()).unwrap();
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            false,
            SessionHeaderType::ChannelJoinRequest,
            0,
            payload,
        );

        request.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());

        let mut msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
        assert_eq!(msg, request);

        msg.set_incoming_conn(Some(conn));
        let msg_id = msg.get_id();
        cp.on_message(msg).await.unwrap();

        // the first message is the set route for moderator name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(
            &participant,
            moderator.agent_type(),
            moderator.agent_id_option(),
            header,
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            false,
            SessionHeaderType::ChannelJoinReply,
            msg_id,
            vec![],
        );
        let mut msg = participant_rx.recv().await.unwrap().unwrap();

        // the payload of the message contains the keypackage and it change all the times
        // so we can compare only the header
        assert_eq!(msg.get_slim_header(), reply.get_slim_header());
        assert_eq!(msg.get_session_header(), reply.get_session_header());

        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await.unwrap();

        // create a reply to compare with the output of on_message
        let mut reply = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            false,
            SessionHeaderType::ChannelMlsWelcome,
            0,
            vec![],
        );

        // this should be the MLS welcome message, we can comprare only
        // the headers like in the previous case
        let mut msg = moderator_rx.recv().await.unwrap().unwrap();
        reply.set_message_id(msg.get_id());
        assert_eq!(msg.get_slim_header(), reply.get_slim_header());
        assert_eq!(msg.get_session_header(), reply.get_session_header());

        // receive the message on the participant side
        msg.set_incoming_conn(Some(conn));
        let msg_id = msg.get_id();
        cp.on_message(msg).await.unwrap();

        // the first message generated is a subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // the third is the ack
        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            false,
            SessionHeaderType::ChannelMlsAck,
            msg_id,
            vec![],
        );

        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);
    }
}
