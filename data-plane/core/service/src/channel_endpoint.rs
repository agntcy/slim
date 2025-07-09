// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use bincode::{Decode, Encode};
use slim_mls::mls::Mls;

use async_trait::async_trait;
use tokio::sync::Mutex;

use tracing::{debug, error, trace};

use crate::{
    errors::{ChannelEndpointError, SessionError},
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
    async fn on_message(&mut self, msg: Message);
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
    pub async fn on_message(&mut self, msg: Message) {
        match self {
            ChannelEndpoint::ChannelParticipant(cp) => {
                cp.on_message(msg).await;
            }
            ChannelEndpoint::ChannelModerator(cm) => {
                cm.on_message(msg).await;
            }
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
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) async fn new(mls: Arc<Mutex<Mls<P, V>>>) -> Result<Self, ChannelEndpointError> {
        mls.lock()
            .await
            .initialize()
            .await
            .map_err(|e| ChannelEndpointError::MLSInit(e.to_string()))?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_commit_id: 0,
        })
    }

    async fn init_moderator(&mut self) -> Result<(), ChannelEndpointError> {
        self.mls
            .lock()
            .await
            .create_group()
            .map(|_| ())
            .map_err(|e| ChannelEndpointError::MLSInit(e.to_string()))
    }

    async fn generate_key_package(&mut self) -> Result<Vec<u8>, ChannelEndpointError> {
        self.mls
            .lock()
            .await
            .generate_key_package()
            .map_err(|e| ChannelEndpointError::MLSInit(e.to_string()))
    }

    async fn process_welcome_message(&mut self, msg: &Message) -> Result<(), ChannelEndpointError> {
        if self.last_commit_id != 0 {
            debug!("welcome message already received, drop");
            // we already got a welcome message, ignore this one
            return Ok(());
        }

        match msg.get_metadata(METADATA_MLS_INIT_COMMIT_ID) {
            Some(x) => {
                debug!("received valid welcome message");
                self.last_commit_id = x.parse::<u32>().unwrap();
            }
            None => {
                error!("received welcome message without commit id, drop it");
                return Err(ChannelEndpointError::WelcomeMessage);
            }
        }

        let welcome = match msg.get_payload() {
            Some(content) => &content.blob,
            None => {
                error!("missing payload in MLS welcome, cannot join the group");
                return Err(ChannelEndpointError::WelcomeMessage);
            }
        };

        match self.mls.lock().await.process_welcome(welcome) {
            Ok(id) => {
                self.group = id;
                Ok(())
            }
            Err(e) => {
                error!("error parsing welcome message {}", e.to_string());
                Err(ChannelEndpointError::WelcomeMessage)
            }
        }
    }

    async fn process_commit_message(&mut self, msg: &Message) -> Result<(), ChannelEndpointError> {
        // the first message to be received should be a welcome message
        // this message will init the last_commit_id. so if last_commit_id = 0
        // drop the commits
        if self.last_commit_id == 0 {
            error!("welcome message not received yet, drop commit");
            return Err(ChannelEndpointError::CommitMessage);
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
            return Err(ChannelEndpointError::CommitMessage);
        }

        let commit = match msg.get_payload() {
            Some(content) => &content.blob,
            None => {
                error!("missing payload in MLS welcome, cannot join the group");
                return Err(ChannelEndpointError::CommitMessage);
            }
        };

        match self.mls.lock().await.process_commit(commit) {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("error processing commit message {}", e.to_string());
                Err(ChannelEndpointError::CommitMessage)
            }
        }
    }

    async fn add_participant(
        &self,
        msg: &Message,
    ) -> Result<(Vec<u8>, Vec<u8>), ChannelEndpointError> {
        let payload = match msg.get_payload() {
            Some(p) => &p.blob,
            None => {
                error!("The key package is missing. the end point cannot be added to the channel");
                return Err(ChannelEndpointError::AddParticipant);
            }
        };

        match self.mls.lock().await.add_member(payload) {
            Ok((commit_payload, welcome_payload)) => Ok((commit_payload, welcome_payload)),
            Err(e) => {
                error!("error adding new endpoint {}", e.to_string());
                Err(ChannelEndpointError::AddParticipant)
            }
        }
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
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        mls_state: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        Endpoint {
            name: name.clone(),
            channel_name: channel_name.clone(),
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
            let f = SlimHeaderFlags::new(10, None, None, None, None);
            Some(f)
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

    async fn join(&mut self) {
        // subscribe only once to the channel
        if self.subscribed {
            return;
        }

        self.subscribed = true;

        // subscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let sub = Message::new_subscribe(&self.name, &self.channel_name, None, header);

        self.send(sub).await;

        // set rout for the channel
        self.set_route(&self.channel_name, None).await;
    }

    async fn set_route(&self, route_name: &AgentType, route_id: Option<u64>) {
        // send a message with subscription from
        let msg = Message::new_subscribe(
            &self.name,
            route_name,
            route_id,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await;
    }

    async fn leave(&self) {
        // unsubscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let unsub = Message::new_unsubscribe(&self.name, &self.channel_name, None, header);

        self.send(unsub).await;
    }

    async fn send(&self, msg: Message) {
        if self.tx.send_to_slim(Ok(msg)).await.is_err() {
            error!("error sending message to slim from channel manager");
            self.tx
                .send_to_app(Err(SessionError::Processing(
                    "error sending message to local slim instance".to_string(),
                )))
                .await
                .expect("error notifying app");
        }
    }
}

#[derive(Debug)]
pub struct ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    endpoint: Endpoint<P, V, T>,
}

impl<P, V, T> ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, mls, tx);
        ChannelParticipant { endpoint }
    }

    async fn on_join_request(&mut self, msg: Message) {
        // get the payload
        let names = match msg.get_payload() {
            Some(content) => {
                let c: JoinMessagePayload = match bincode::decode_from_slice(
                    &content.blob,
                    bincode::config::standard(),
                ) {
                    Ok(c) => c.0,
                    Err(_) => {
                        error!(
                            "error decoding payload in a Join Channel request, ignore the message"
                        );
                        return;
                    }
                };
                // channel name
                c
            }
            None => {
                error!("missing payload in a Join Channel request, ignore the message");
                return;
            }
        };

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
            .await;

        // send reply to the moderator
        let src = msg.get_source();
        let payload: Vec<u8> = if msg.contains_metadata(METADATA_MLS_ENABLED) {
            // if mls we need to provide the key package
            match self.endpoint.mls_state.as_mut() {
                Some(mls_state) => match mls_state.generate_key_package().await {
                    Ok(key_package) => key_package,
                    Err(_) => {
                        error!("error generating key package, drop join request");
                        return;
                    }
                },
                None => {
                    error!("MLS not initialized, cannot generate key package");
                    return;
                }
            }
        } else {
            // without MLS we can set the state for the channel
            // otherwise the endpoint needs to receive a
            // welcome message first
            self.endpoint.join().await;
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

        self.endpoint.send(reply).await;
    }

    async fn on_mls_welcome(&mut self, msg: Message) {
        match self.endpoint.mls_state.as_mut() {
            Some(mls_state) => match mls_state.process_welcome_message(&msg).await {
                Ok(()) => {}
                Err(_) => {
                    // error processing welcome message, drop it
                    return;
                }
            },
            None => {
                error!("MLS not initialized, cannot process welcome message");
                return;
            }
        }

        debug!("Welcome message correctly processed, MLS state initialized");

        // set route for the channel name
        self.endpoint.join().await;

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
        self.endpoint.send(ack).await;
    }

    async fn on_mls_commit(&mut self, msg: Message) {
        match self.endpoint.mls_state.as_mut() {
            Some(mls_state) => match mls_state.process_commit_message(&msg).await {
                Ok(()) => {}
                Err(_) => {
                    // error processing commit message, drop it
                    return;
                }
            },
            None => {
                error!("MLS not initialized, cannot process commit message");
                return;
            }
        }

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
        self.endpoint.send(ack).await;
    }
}

impl<P, V, T> OnMessageReceived for ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                error!(
                    "Received discovery request message, this should not happen. drop the message"
                );
            }
            SessionHeaderType::ChannelJoinRequest => {
                debug!("Received join request message");
                self.on_join_request(msg).await;
            }
            SessionHeaderType::ChannelMlsWelcome => {
                debug!("Received mls welcome message");
                self.on_mls_welcome(msg).await;
            }
            SessionHeaderType::ChannelMlsCommit => {
                debug!("Received mls commit message");
                self.on_mls_commit(msg).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                debug!("Received leave request message");
                // leave the channell
                self.endpoint.leave().await;

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

                self.endpoint.send(reply).await;
            }
            _ => {
                error!("received unexpected packet type");
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
    channel_list: HashSet<Agent>,

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
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let p = JoinMessagePayload::new(channel_name.clone(), name.clone());
        let invite_payload: Vec<u8> = bincode::encode_to_vec(p, bincode::config::standard())
            .expect("unable to parse channel name as payload");

        let endpoint = Endpoint::new(name, channel_name, session_id, mls, tx);
        ChannelModerator {
            endpoint,
            channel_list: HashSet::new(),
            pending_requests: HashMap::new(),
            max_retries,
            retries_interval,
            invite_payload,
            mls_msg_id: 0,
        }
    }

    pub async fn join(&mut self) {
        if !self.endpoint.subscribed {
            // join the channel
            self.endpoint.join().await;

            // create mls group if needed
            match self.endpoint.mls_state.as_mut() {
                Some(mls_state) => match mls_state.init_moderator().await {
                    Ok(()) => {
                        debug!("MLS group created successfully");
                    }
                    Err(e) => {
                        error!("error creating MLS group: {}", e);
                        return;
                    }
                },
                None => {
                    error!("MLS not initialized, cannot create group");
                    return;
                }
            }

            // add the moderator to the channel
            self.channel_list.insert(self.endpoint.name.clone());
        }
    }

    async fn forward(&mut self, msg: Message) {
        // forward message received from the app and set a timer
        let msg_id = msg.get_id();
        self.endpoint.send(msg.clone()).await;
        // create a timer for this request
        self.create_timer(msg_id, 1, msg);
    }

    fn create_timer(&mut self, key: u32, pending_mesasges: u32, msg: Message) {
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

        self.pending_requests.insert(key, (timer, pending_mesasges));
    }

    fn delete_timer(&mut self, key: u32) -> bool {
        match self.pending_requests.get_mut(&key) {
            Some((t, p)) => {
                *p -= 1;
                if *p == 0 {
                    t.stop();
                    self.pending_requests.remove(&key);
                    return true;
                }
            }
            None => {
                error!("received a reply from unknown agent, drop message");
            }
        }
        false
    }

    fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.mls_msg_id += 1;
        self.mls_msg_id
    }

    async fn on_discovery_reply(&mut self, msg: Message) {
        // set the local state and join the channel
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.join().await;

        // an endpoint replyed to the discovery message
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

        self.endpoint.send(join.clone()).await;

        // remove the timer for the discovery message
        self.delete_timer(recv_msg_id);

        // add a new one for the join message
        self.create_timer(new_msg_id, 1, join);
    }

    async fn on_join_reply(&mut self, msg: Message) {
        let src = msg.get_slim_header().get_source();
        let msg_id = msg.get_id();

        // cancel timer, there only one message pending here
        self.delete_timer(msg_id);

        // send MLS messages if needed
        if self.endpoint.mls_state.is_some() {
            let (commit_payload, welcome_payload) = match self
                .endpoint
                .mls_state
                .as_ref()
                .unwrap()
                .add_participant(&msg)
                .await
            {
                Ok((commit_payload, welcome_payload)) => (commit_payload, welcome_payload),
                Err(_) => {
                    // error adding participant, drop message
                    return;
                }
            };

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
            self.endpoint.send(welcome.clone()).await;
            self.create_timer(welcome_id, 1, welcome);

            // send commit message if needed
            if self.channel_list.len() > 1 {
                debug!("Send MLS Commit Message to the channel");
                self.endpoint.send(commit.clone()).await;
                self.create_timer(
                    commit_id,
                    (self.channel_list.len() - 1).try_into().unwrap(),
                    commit,
                );
            }
        };

        // track source in the channel list
        self.channel_list.insert(src);
    }

    async fn on_msl_ack(&mut self, msg: Message) {
        let recv_msg_id = msg.get_id();
        self.delete_timer(recv_msg_id);
    }
}

impl<P, V, T> OnMessageReceived for ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                debug!("Invite new participant to the channel, send discovery message");
                // discovery message coming from the application
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelDiscoveryReply => {
                debug!("Received discovery reply message");
                self.on_discovery_reply(msg).await;
            }
            SessionHeaderType::ChannelJoinReply => {
                debug!("Received join reply message");
                self.on_join_reply(msg).await;
            }
            SessionHeaderType::ChannelMlsAck => {
                debug!("Received mls ack message");
                self.on_msl_ack(msg).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                // leave message coming from the application
                debug!("Received leave request message");
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelLeaveReply => {
                debug!("Received leave reply message");
                let src = msg.get_slim_header().get_source();
                let msg_id = msg.get_id();

                // cancel timer
                self.delete_timer(msg_id);

                // remove from the channel list
                self.channel_list.remove(&src);
            }
            _ => {
                error!("received unexpected packet type");
            }
        }
    }
}

/*#[cfg(test)]
mod tests {
    use crate::testutils::MockTransmitter;

    use super::*;
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
        let conn = 1;

        let mut cm = ChannelModerator::new(
            &moderator,
            &channel_name,
            SESSION_ID,
            conn,
            3,
            Duration::from_millis(100),
            moderator_tx,
        );
        let mut cp = ChannelParticipant::new(
            &participant,
            &channel_name,
            SESSION_ID,
            conn,
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
        cm.on_message(request.clone()).await;

        // the request is forwarded to slim
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        assert_eq!(request, msg);

        // the message is received by the participant
        cp.on_message(msg).await;

        // the first message received  by slim should be a set route for the moderator name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(
            &participant,
            moderator.agent_type(),
            moderator.agent_id_option(),
            header,
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create the expected reply for comparison
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            SessionHeaderType::ChannelDiscoveryReply,
            msg_id,
            vec![],
        );

        let mut msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);

        // message reception on moderator side
        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await;

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
        let payload: Vec<u8> =
            bincode::encode_to_vec(&channel_name, bincode::config::standard()).unwrap();
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            SessionHeaderType::ChannelJoinRequest,
            0,
            payload,
        );

        let mut msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
        assert_eq!(msg, request);

        msg.set_incoming_conn(Some(conn));
        let msg_id = msg.get_id();
        cp.on_message(msg).await;

        // the first message is the subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            SessionHeaderType::ChannelJoinReply,
            msg_id,
            vec![],
        );
        let mut msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);

        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await;

        // end with the message exchange check the state
        assert_eq!(cm.channel_list.len(), 2);
        assert!(cm.channel_list.contains(&moderator));
        assert!(cm.channel_list.contains(&participant));
        assert_eq!(cm.pending_requests.len(), 0);

        // ask to leave to the participant
        // create a request to remove paticipant from the group
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            SessionHeaderType::ChannelLeaveRequest,
            0,
            vec![],
        );
        request.set_incoming_conn(Some(conn));

        cm.on_message(request.clone()).await;

        // get discovery request
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        //request.set_message_id(msg_id);
        assert_eq!(msg, request);

        cp.on_message(msg).await;

        // the firs message will be the unsubscribe
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let unsub = Message::new_unsubscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, unsub);

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            SessionHeaderType::ChannelLeaveReply,
            msg_id,
            vec![],
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);

        // mimic message reception on moderator side
        cm.on_message(msg).await;

        // nothing should happen except that the participant
        // is removed by the group
        assert_eq!(cm.channel_list.len(), 1);
        assert!(cm.channel_list.contains(&moderator));
        assert!(!cm.channel_list.contains(&participant));
        assert_eq!(cm.pending_requests.len(), 0);
    }
}*/
