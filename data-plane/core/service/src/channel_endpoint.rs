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

use tracing::{debug, error, info, trace};

use crate::{
    errors::{ChannelEndpointError, SessionError},
    session::{Id, SessionTransmitter},
};

use slim_datapath::{
    api::{
        SessionHeader, SlimHeader,
        proto::pubsub::v1::{Message, SessionHeaderType},
    },
    messages::{Agent, AgentType, utils::SlimHeaderFlags},
};

use crate::interceptor_mls::METADATA_MLS_ENABLED;
use crate::interceptor_mls::METADATA_MLS_INIT_COMMIT_ID;

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
pub enum ChannelEndpoint<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    ChannelParticipant(ChannelParticipant<T>),
    ChannelModerator(ChannelModerator<T>),
}

impl<T> ChannelEndpoint<T>
where
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
struct MlsState {
    /// true if initialized
    init: bool,

    /// mls state for the channel of this endpoint
    /// the mls state should be created and initiaded in the app
    /// so that it can be shared with the channel and the interceptors
    mls: Option<Arc<Mutex<Mls>>>,

    /// used only is Some(mls)
    group: Vec<u8>,

    /// last commit id
    last_commit_id: u32,
}

impl MlsState {
    async fn init(&mut self) -> Result<(), ChannelEndpointError> {
        if self.init {
            return Ok(());
        }

        if let Some(mls) = &self.mls {
            let mut lock = mls.lock().await;
            match lock.initialize().await {
                Ok(()) => {
                    self.init = true;
                    return Ok(());
                }
                Err(e) => {
                    return Err(ChannelEndpointError::MLSInit(e.to_string()));
                }
            }
        };

        Err(ChannelEndpointError::NoMls)
    }

    async fn init_moderator(&mut self) -> Result<(), ChannelEndpointError> {
        self.init().await?;

        self.group = match &self.mls {
            Some(mls) => {
                let mut lock = mls.lock().await;
                match lock.create_group() {
                    Ok(id) => id,
                    Err(e) => {
                        error!("error creating a new group {}", e.to_string());
                        return Err(ChannelEndpointError::MLSInit(e.to_string()));
                    }
                }
            }
            None => vec![],
        };

        Ok(())
    }
    async fn generate_key_package(&mut self) -> Result<Vec<u8>, ChannelEndpointError> {
        self.init().await?;

        if let Some(mls) = &self.mls {
            let lock = mls.lock().await;
            match lock.generate_key_package() {
                Ok(msg) => {
                    return Ok(msg);
                }
                Err(e) => {
                    return Err(ChannelEndpointError::MLSKeyPackage(e.to_string()));
                }
            }
        };

        Err(ChannelEndpointError::NoMls)
    }

    async fn process_welcome_message(&mut self, msg: &Message) -> Result<(), ChannelEndpointError> {
        if !self.init {
            return Err(ChannelEndpointError::NoMls);
        }

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

        match &self.mls {
            Some(mls) => {
                let mut lock = mls.lock().await;
                match lock.process_welcome(welcome) {
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
            None => {
                error!("no mls state set. cannot process the welcome message");
                Err(ChannelEndpointError::WelcomeMessage)
            }
        }
    }

    async fn process_commit_message(&mut self, msg: &Message) -> Result<(), ChannelEndpointError> {
        if !self.init {
            return Err(ChannelEndpointError::NoMls);
        }

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

        match &self.mls {
            Some(mls) => {
                let mut lock = mls.lock().await;
                match lock.process_commit(commit) {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        error!("error processing commit message {}", e.to_string());
                        Err(ChannelEndpointError::CommitMessage)
                    }
                }
            }
            None => {
                error!("MLS not setup, drop commit message");
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

        match &self.mls {
            Some(mls) => {
                let mut lock = mls.lock().await;
                match lock.add_member(payload) {
                    Ok((commit_payload, welcome_payload)) => Ok((commit_payload, welcome_payload)),
                    Err(e) => {
                        error!("error adding new endpoint {}", e.to_string());
                        Err(ChannelEndpointError::AddParticipant)
                    }
                }
            }
            None => {
                error!("MLS not seutp, drop commit message");
                Err(ChannelEndpointError::CommitMessage)
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

struct Endpoint<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// endpoint name
    name: Agent,

    /// channel name
    channel_name: AgentType,

    /// id of the current session
    session_id: Id,

    /// connection id to the next hop SLIM
    conn: u64,

    /// true is the endpoint is already subscribed to the channel
    subscribed: bool,

    /// mls state
    mls_state: MlsState,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

impl<T> Endpoint<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        mls: Option<Arc<Mutex<Mls>>>,
        tx: T,
    ) -> Self {
        Endpoint {
            name: name.clone(),
            channel_name: channel_name.clone(),
            session_id,
            conn,
            subscribed: false,
            mls_state: MlsState {
                init: false,
                mls,
                group: vec![],
                last_commit_id: 0,
            },
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
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
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
            Some(SlimHeaderFlags::default().with_recv_from(self.conn)),
        );

        self.send(msg).await;
    }

    async fn leave(&self) {
        // unsubscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
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
pub struct ChannelParticipant<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    endpoint: Endpoint<T>,
}

impl<T> ChannelParticipant<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        mls: Option<Arc<Mutex<Mls>>>,
        tx: T,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, mls, tx);
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
        self.endpoint.conn = msg.get_incoming_conn();
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
            info!("MLS is required, reply with key package");
            match &self.endpoint.mls_state.generate_key_package().await {
                Ok(payload) => payload.to_vec(),
                Err(e) => {
                    error!(
                        "received a join request with MLS, error creating the key package {}",
                        e.to_string()
                    );
                    // ignore the request and return
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
        match self.endpoint.mls_state.process_welcome_message(&msg).await {
            Ok(()) => {}
            Err(_) => {
                //error processing welcome message, drop it
                return;
            }
        }

        info!("Welcome message correctly processed, MLS state initialized");

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
        match self.endpoint.mls_state.process_commit_message(&msg).await {
            Ok(()) => {}
            Err(_) => {
                //error processing commit message, drop it
                return;
            }
        }

        info!("Commit message correctly processed, MLS state updated");

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

impl<T> OnMessageReceived for ChannelParticipant<T>
where
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
                info!("Received join request message");
                self.on_join_request(msg).await;
            }
            SessionHeaderType::ChannelMlsWelcome => {
                info!("Received mls welcome message");
                self.on_mls_welcome(msg).await;
            }
            SessionHeaderType::ChannelMlsCommit => {
                info!("Received mls commit message");
                self.on_mls_commit(msg).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                info!("Received leave request message");
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
pub struct ChannelModerator<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    endpoint: Endpoint<T>,

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
impl<T> ChannelModerator<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<Arc<Mutex<Mls>>>,
        tx: T,
    ) -> Self {
        let p = JoinMessagePayload::new(channel_name.clone(), name.clone());
        let invite_payload: Vec<u8> = bincode::encode_to_vec(p, bincode::config::standard())
            .expect("unable to parse channel name as payload");

        let endpoint = Endpoint::new(name, channel_name, session_id, conn, mls, tx);
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
            match self.endpoint.mls_state.init_moderator().await {
                Ok(()) => {}
                Err(_) => {
                    // error while init the moderator. return;
                    return;
                }
            }

            // add the moderator to the channel
            self.channel_list.insert(self.endpoint.name.clone());
        }
    }

    async fn forward(&mut self, msg: Message) {
        // forward message received form the app and set a timer
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
        self.endpoint.conn = msg.get_incoming_conn();
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

        if self.endpoint.mls_state.mls.is_some() {
            join.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());
            info!("Reply with the join request, MLS is enabled");
        } else {
            info!("Reply with the join request, MLS is disabled");
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
        if self.endpoint.mls_state.mls.is_some() {
            info!("Generate MLS Welcome and Commit Message");
            let (commit_payload, welcome_payload) =
                match self.endpoint.mls_state.add_participant(&msg).await {
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
            info!("Send MLS Welcome Message to the new participant");
            self.endpoint.send(welcome.clone()).await;
            self.create_timer(welcome_id, 1, welcome);

            // send commit message if needed
            if self.channel_list.len() > 1 {
                info!("Send MLS Commit Message to the channel");
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

impl<T> OnMessageReceived for ChannelModerator<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                info!("Invite new participant to the channel, send discovery message");
                // discovery message coming from the application
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelDiscoveryReply => {
                info!("Received discovery reply message");
                self.on_discovery_reply(msg).await;
            }
            SessionHeaderType::ChannelJoinReply => {
                info!("Received join reply message");
                self.on_join_reply(msg).await;
            }
            SessionHeaderType::ChannelMlsAck => {
                info!("Received mls ack message");
                self.on_msl_ack(msg).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                // leave message coming from the application
                info!("Received leave request message");
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelLeaveReply => {
                info!("Received leave reply message");
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

#[cfg(test)]
mod tests {
    use crate::testutils::MockTransmitter;

    use super::*;
    use slim_mls::identity::FileBasedIdentityProvider;
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

        let moderator_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/moderator").unwrap());
        let participant_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/participant").unwrap());

        let mut cm = ChannelModerator::new(
            &moderator,
            &channel_name,
            SESSION_ID,
            conn,
            3,
            Duration::from_millis(100),
            Some(Arc::new(Mutex::new(Mls::new(
                "moderator".to_string(),
                moderator_provider,
            )))),
            moderator_tx,
        );
        let mut cp = ChannelParticipant::new(
            &participant,
            &channel_name,
            SESSION_ID,
            conn,
            Some(Arc::new(Mutex::new(Mls::new(
                "participant".to_string(),
                participant_provider,
            )))),
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
        cp.on_message(msg).await;

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
        cm.on_message(msg).await;

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
        cp.on_message(msg).await;

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
