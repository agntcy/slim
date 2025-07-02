// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;

use tracing::{debug, error, trace};

use crate::{
    errors::SessionError,
    session::{AppChannelSender, Id, SlimChannelSender},
};
use slim_datapath::{
    api::{
        SessionHeader, SlimHeader,
        proto::pubsub::v1::{Message, SessionHeaderType},
    },
    messages::{Agent, AgentType, utils::SlimHeaderFlags},
};

struct RequestTimerObserver {
    /// message to send in case of timeout
    message: Message,

    /// channel to the local slim instance
    send_slim: SlimChannelSender,

    /// channel to send messages to the application (used to signal errors)
    send_app: AppChannelSender,
}

#[async_trait]
impl crate::timer::TimerObserver for RequestTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for request {}", timeouts, timer_id);

        if self.send_slim.send(Ok(self.message.clone())).await.is_err() {
            error!("error sending invite message");
        }
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        error!("unable to send message {:?}, stop retrying", self.message);
        self.send_app
            .send(Err(SessionError::Processing(
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
pub enum ChannelEndpoint {
    ChannelParticipant(ChannelParticipant),
    ChannelModerator(ChannelModerator),
}

impl ChannelEndpoint {
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
struct Endpoint {
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

    /// channel to send messages to the local slim instance
    send_slim: SlimChannelSender,

    /// channel to send messages to the application (used to signal errors)
    send_app: AppChannelSender,
}

impl Endpoint {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
        send_app: AppChannelSender,
    ) -> Self {
        Endpoint {
            name: name.clone(),
            channel_name: channel_name.clone(),
            session_id,
            conn,
            subscribed: false,
            send_slim,
            send_app,
        }
    }

    fn create_channel_message(
        &self,
        destination: &AgentType,
        destination_id: Option<u64>,
        request_type: SessionHeaderType,
        message_id: u32,
        payload: Vec<u8>,
    ) -> Message {
        let slim_header = Some(SlimHeader::new(
            &self.name,
            destination,
            destination_id,
            None,
        ));

        let session_header = Some(SessionHeader::new(
            request_type.into(),
            self.session_id,
            message_id,
        ));

        Message::new_publish_with_headers(slim_header, session_header, "", payload)
    }

    async fn join(&self) {
        // subscribe only once to the channel
        if self.subscribed {
            return;
        }

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
        if self.send_slim.send(Ok(msg)).await.is_err() {
            error!("error sending message to slim from channel manager");
            self.send_app
                .send(Err(SessionError::Processing(
                    "error sending message to local slim instance".to_string(),
                )))
                .await
                .expect("error notifying app");
        }
    }
}

#[derive(Debug)]
pub struct ChannelParticipant {
    endpoint: Endpoint,
}

impl ChannelParticipant {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType, // TODO: this may be unknown at the beginning, probably it has to be optional
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
        send_app: AppChannelSender,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, send_slim, send_app);
        ChannelParticipant { endpoint }
    }
}

impl OnMessageReceived for ChannelParticipant {
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                debug!("received discovery request message");
                // set local state according to the info in the message
                self.endpoint.conn = msg.get_incoming_conn();
                self.endpoint.session_id = msg.get_session_header().get_session_id();

                // get the source (with strings) from the packet payload
                let source = match msg.get_payload() {
                    Some(content) => {
                        let c: Agent = match bincode::decode_from_slice(
                            &content.blob,
                            bincode::config::standard(),
                        ) {
                            Ok(c) => c.0,
                            Err(_) => {
                                error!(
                                    "error decoding payload in a Discovery Channel request, ignore the message"
                                );
                                return;
                            }
                        };
                        // channel name
                        c
                    }
                    None => {
                        error!(
                            "missing payload in a Discovery Channel request, ignore the message"
                        );
                        return;
                    }
                };

                // set route in order to be able to send packets to the moderator
                self.endpoint
                    .set_route(source.agent_type(), source.agent_id_option())
                    .await;

                // set the connection id equal to the connection from where we received the message
                let src = msg.get_source();

                // create reply message
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelDiscoveryReply,
                    msg.get_id(),
                    vec![],
                );

                self.endpoint.send(reply).await;
            }
            SessionHeaderType::ChannelJoinRequest => {
                debug!("received join request message");
                // accept the join and set the state
                // add the group name to the local endpoint and subscribe
                let src = msg.get_source();

                let channel_name = match msg.get_payload() {
                    Some(content) => {
                        let c: AgentType = match bincode::decode_from_slice(
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

                // join the channel
                self.endpoint.channel_name = channel_name;
                self.endpoint.join().await;

                // reply to the request
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelJoinReply,
                    msg.get_id(),
                    vec![],
                );
                self.endpoint.send(reply).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                debug!("received leave request message");
                // leave the channell
                self.endpoint.leave().await;

                // reply to the request
                let src = msg.get_source();
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
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
pub struct ChannelModerator {
    endpoint: Endpoint,

    /// list of endpoint names in the channel
    channel_list: HashSet<Agent>,

    /// list of pending requests and related timers
    pending_requests: HashMap<u32, crate::timer::Timer>,

    /// number or maximum retries before give up with a control message
    max_retries: u32,

    /// interval between retries
    retries_interval: Duration,

    /// channel name as payload to add to the invite messages
    payload: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
impl ChannelModerator {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        max_retries: u32,
        retries_interval: Duration,
        send_slim: SlimChannelSender,
        send_app: AppChannelSender,
    ) -> Self {
        let payload: Vec<u8> = bincode::encode_to_vec(channel_name, bincode::config::standard())
            .expect("unable to parse channel name as payload");
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, send_slim, send_app);
        ChannelModerator {
            endpoint,
            channel_list: HashSet::new(),
            pending_requests: HashMap::new(),
            max_retries,
            retries_interval,
            payload,
        }
    }

    pub async fn join(&mut self) {
        if !self.endpoint.subscribed {
            // join the channel
            self.endpoint.join().await;

            // add the moderator to the channel
            self.channel_list.insert(self.endpoint.name.clone());
        }
    }

    async fn forward(&mut self, msg: Message) {
        // forward message received form the app and set a timer
        let msg_id = msg.get_id();
        self.endpoint.send(msg.clone()).await;
        // create a timer for this request
        self.create_timer(msg_id, msg);
    }

    fn create_timer(&mut self, key: u32, msg: Message) {
        let observer = Arc::new(RequestTimerObserver {
            message: msg,
            send_slim: self.endpoint.send_slim.clone(),
            send_app: self.endpoint.send_app.clone(),
        });

        let timer = crate::timer::Timer::new(
            key,
            crate::timer::TimerType::Constant,
            self.retries_interval,
            None,
            Some(self.max_retries),
        );
        timer.start(observer);

        self.pending_requests.insert(key, timer);
    }

    fn delete_timer(&mut self, key: u32) {
        match self.pending_requests.get_mut(&key) {
            Some(t) => {
                t.stop();
                self.pending_requests.remove(&key);
            }
            None => {
                error!("received a reply from unknown agent, drop message");
            }
        }
    }
}

impl OnMessageReceived for ChannelModerator {
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                debug!("received discovery request message from app");
                // discovery message coming from the application
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelDiscoveryReply => {
                debug!("received discovery reply message");
                // set the local state and join the channel
                self.endpoint.conn = msg.get_incoming_conn();
                self.join().await;

                // an endpoint replyed to the discovery message
                // send a join message
                let src = msg.get_slim_header().get_source();
                let recv_msg_id = msg.get_id();
                let new_msg_id = rand::random::<u32>();

                // this is the only message that cannot be received but that is created here
                let join = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelJoinRequest,
                    new_msg_id,
                    self.payload.clone(),
                );

                self.endpoint.send(join.clone()).await;

                // remove the timer for the discovery message
                self.delete_timer(recv_msg_id);

                // add a new one for the join message
                self.create_timer(new_msg_id, join);
            }
            SessionHeaderType::ChannelJoinReply => {
                debug!("received join reply message");

                let src = msg.get_slim_header().get_source();
                let msg_id = msg.get_id();

                // cancel timer
                self.delete_timer(msg_id);

                // add source to the channel
                self.channel_list.insert(src);
            }
            SessionHeaderType::ChannelLeaveRequest => {
                // leave message coming from the application
                debug!("received leave request message");
                self.forward(msg).await;
            }
            SessionHeaderType::ChannelLeaveReply => {
                debug!("received leave reply message");
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
            tx_app.clone(),
        );
        let mut cp = ChannelParticipant::new(
            &participant,
            &channel_name,
            SESSION_ID,
            conn,
            participant_tx,
            tx_app.clone(),
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
}
