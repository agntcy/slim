// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use tonic::async_trait;

use tracing::{debug, error, trace};

use crate::session::{Id, SlimChannelSender};
use slim_datapath::{
    api::{
        SessionHeader, SlimHeader,
        proto::pubsub::v1::{Message, SessionHeaderType},
    },
    messages::{Agent, AgentType, encoder::DEFAULT_AGENT_ID, utils::SlimHeaderFlags},
};

#[allow(dead_code)]
struct RequestTimerObserver {
    message: Message,
    send_slim: SlimChannelSender,
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
        // TO DO what do we do here ??
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!("timer for rtx {} cancelled", timer_id);
        // nothing to do
    }
}

enum ChannelEndpoint {
    ChannelParticipant,
    ChannelModerator,
}

trait OnMessageReceived {
    fn on_message(&mut self, msg: Message);
}

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
}

impl Endpoint {
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
    ) -> Self {
        Endpoint { name, channel_name, session_id, conn, subscribed: false, send_slim }
    }

    async fn join(&self) {
        // subscribe only once to the channel
        if self.subscribed {
            return;
        }

        // subscribe for the topic
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
        let sub = Message::new_subscribe(&self.name, &self.channel_name, None, header);

        // TODO
        // handle error
        self.send(sub).await;
    }

    async fn leave(&self) {
        // unsubscribe for the topic
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
        let sub = Message::new_unsubscribe(&self.name, &self.channel_name, None, header);

        // TODO
        // handle error
        self.send(sub).await;
    }

    async fn send(&self, msg: Message) {
        // TODO
         // handle error
        if self.send_slim.send(Ok(msg)).await.is_err() {
            error!("error sending invite message");
        }
    }
}

struct ChannelParticipant {
    endpoint: Endpoint,
}

impl ChannelParticipant {
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, send_slim);
        ChannelParticipant {endpoint}
    }
}

impl OnMessageReceived for ChannelParticipant {
    fn on_message(&mut self, msg: Message) {
        todo!()
    }
}

#[allow(dead_code)]
struct ChannelModerator {
    endpoint: Endpoint,

    /// list of endpoint names in the channel
    channel_list: HashSet<Agent>,

    /// list of pending requests and related timers
    pending_requests: HashMap<Agent, crate::timer::Timer>,

    /// number or maximum retries before give up with a control message
    max_reties: u32,

    /// interval between retries
    retries_interval: Duration,
}

#[allow(dead_code)]
impl ChannelModerator {
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        session_id: Id,
        conn: u64,
        max_reties: u32,
        retries_interval: Duration,
        send_slim: SlimChannelSender,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, send_slim);
        ChannelModerator {
            endpoint,
            channel_list: HashSet::new(),
            pending_requests: HashMap::new(),
            max_reties,
            retries_interval,
        }
    }

    pub async fn invite(&mut self, agent: &AgentType) {
        // join the channel if needed
        self.endpoint.join().await;

        // create a join message to invite a new endpoint to the group
        let join = self.create_request(agent, None, SessionHeaderType::JoinRequest);

        // TODO
        // return an error
        self.endpoint.send(join.clone()).await;

        // create a timer for this request
        let a = Agent::new(agent.clone(), DEFAULT_AGENT_ID);
        self.create_timer(a, join);
    }

    pub async fn ask_to_leave(&mut self, agent: Agent) {
        let leave = self.create_request(
            agent.agent_type(),
            agent.agent_id_option(),
            SessionHeaderType::LeaveRequest,
        );

        // TODO
        // return an error
        self.endpoint.send(leave.clone()).await;

        // create a timer for this request
        self.create_timer(agent, leave);
    }

    pub async fn delete_group(&mut self) {
        for a in self.channel_list.clone() {
            self.ask_to_leave(a).await;
        }

        self.endpoint.leave().await;
    }

    fn create_request(
        &self,
        agent: &AgentType,
        agent_id: Option<u64>,
        request_type: SessionHeaderType,
    ) -> Message {
        let agp_header = Some(SlimHeader::new(&self.endpoint.name, agent, agent_id, None));

        let session_header = Some(SessionHeader::new(
            request_type.into(),
            self.endpoint.session_id,
            rand::random::<u32>(),
        ));

        let payload = bincode::encode_to_vec(&self.endpoint.channel_name, bincode::config::standard()).unwrap();
        Message::new_publish_with_headers(agp_header, session_header, "", payload)
    }

    fn create_timer(&mut self, name: Agent, msg: Message) {
        let observer = Arc::new(RequestTimerObserver {
            message: msg,
            send_slim: self.endpoint.send_slim.clone(),
        });

        let timer = crate::timer::Timer::new(
            rand::random::<u32>(),
            crate::timer::TimerType::Constant,
            self.retries_interval,
            None,
            Some(self.max_reties),
        );
        timer.start(observer);

        //let a = Agent::new(agent.clone(), DEFAULT_AGENT_ID);
        self.pending_requests.insert(name, timer);
    }
}

impl OnMessageReceived for ChannelModerator {
    fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::JoinReply => {
                debug!("received join reply message");
                let src = msg.get_slim_header().get_source();
                let key = src.clone().with_agent_id(DEFAULT_AGENT_ID);
                match self.pending_requests.get_mut(&key) {
                    Some(t) => {
                        t.stop();
                        self.pending_requests.remove(&src);
                        self.channel_list.insert(src);
                    }
                    None => {
                        error!("received a reply from unknown agent, drop message");
                    }
                }
            }
            SessionHeaderType::LeaveReply => {
                debug!("received leave reply message");
                let src = msg.get_slim_header().get_source();
                match self.pending_requests.get_mut(&src) {
                    Some(t) => {
                        t.stop();
                        self.pending_requests.remove(&src);
                        self.channel_list.remove(&src);
                    }
                    None => {
                        error!("received a reply from unknown agent, drop message");
                    }
                }
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

    const REQ_ID: u32 = 50;
    const SESSION_ID: u32 = 10;

    #[tokio::test]
    #[traced_test]
    async fn test_invite_no_reply() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let source = Agent::from_strings("org", "default", "source", 0);
        let topic = AgentType::from_strings("org", "default", "channel");

        let agent_to_invite = Agent::from_strings("org", "default", "to_invite", 5120);

        let mut cm = ChannelModerator::new(source, topic.clone(), SESSION_ID, 1, 10, Duration::from_secs(1), tx_slim);

        cm.invite(agent_to_invite.agent_type()).await;

        // create a request to compare with the output of the invite
        let mut request = cm.create_request(
            agent_to_invite.agent_type(),
            None,
            SessionHeaderType::JoinRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        for _ in 0..11 {
            let mut msg = rx_slim.recv().await.unwrap().unwrap();
            let payload = &msg.get_payload().unwrap().blob;
            let agent: AgentType =
                bincode::decode_from_slice(payload.as_ref(), bincode::config::standard())
                    .unwrap()
                    .0;
            assert_eq!(agent, topic);

            msg.get_session_header_mut().set_message_id(REQ_ID);
            assert_eq!(msg, request);
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_invite_with_reply() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let source = Agent::from_strings("org", "default", "source", 12345);
        let topic = AgentType::from_strings("org", "channel", "topic");

        let agent_to_invite = Agent::from_strings("org", "default", "to_invite", 5120);

        let mut cm = ChannelModerator::new(source.clone(), topic, SESSION_ID, 1, 10, Duration::from_secs(1), tx_slim);

        cm.invite(agent_to_invite.agent_type()).await;

        let mut request = cm.create_request(
            agent_to_invite.agent_type(),
            None,
            SessionHeaderType::JoinRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        let mut msg = rx_slim.recv().await.unwrap().unwrap();
        msg.get_session_header_mut().set_message_id(REQ_ID);
        assert_eq!(msg, request);

        // create the reply message
        let agp_header = Some(SlimHeader::new(
            &agent_to_invite,
            source.agent_type(),
            source.agent_id_option(),
            None,
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::JoinReply.into(),
            SESSION_ID,
            REQ_ID,
        ));

        let reply = Message::new_publish_with_headers(agp_header, session_header, "", vec![]);

        // receive the reply msg
        cm.on_message(reply);

        cm.ask_to_leave(agent_to_invite.clone()).await;

        let mut request = cm.create_request(
            agent_to_invite.agent_type(),
            agent_to_invite.agent_id_option(),
            SessionHeaderType::LeaveRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        let mut msg = rx_slim.recv().await.unwrap().unwrap();
        msg.get_session_header_mut().set_message_id(REQ_ID);
        assert_eq!(msg, request);

        // create the reply message
        let agp_header = Some(SlimHeader::new(
            &agent_to_invite,
            source.agent_type(),
            source.agent_id_option(),
            None,
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::LeaveReply.into(),
            SESSION_ID,
            REQ_ID,
        ));

        let reply = Message::new_publish_with_headers(agp_header, session_header, "", vec![]);

        // receive the reply msg
        cm.on_message(reply);
    }
}
