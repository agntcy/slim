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

#[allow(dead_code)]
struct ChannelModerator {
    source: Agent,
    topic: AgentType,
    session_id: Id,
    conn: u64,
    group: HashSet<Agent>,
    pending_requests: HashMap<Agent, crate::timer::Timer>,
    send_slim: SlimChannelSender,
}

#[allow(dead_code)]
impl ChannelModerator {
    pub fn new(
        source: Agent,
        topic: AgentType,
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
    ) -> Self {
        ChannelModerator {
            source,
            topic,
            session_id,
            conn,
            group: HashSet::new(),
            pending_requests: HashMap::new(),
            send_slim,
        }
    }

    pub async fn invite(&mut self, agent: &AgentType) {
        let join = self.create_request(agent, None, SessionHeaderType::JoinRequest);

        // TODO
        // return an error
        if self.send_slim.send(Ok(join.clone())).await.is_err() {
            error!("error sending invite message");
            return;
        }

        // create a timer for this request
        let observer = Arc::new(RequestTimerObserver {
            message: join,
            send_slim: self.send_slim.clone(),
        });

        let timer = crate::timer::Timer::new(
            rand::random::<u32>(),
            crate::timer::TimerType::Constant,
            Duration::from_secs(1),
            None,
            Some(10),
        );
        timer.start(observer);

        let a = Agent::new(agent.clone(), DEFAULT_AGENT_ID);
        self.pending_requests.insert(a, timer);
    }

    pub async fn ask_to_leave(&mut self, agent: &Agent) {
        let leave = self.create_request(
            agent.agent_type(),
            agent.agent_id_option(),
            SessionHeaderType::LeaveRequest,
        );

        // TODO
        // return an error
        if self.send_slim.send(Ok(leave.clone())).await.is_err() {
            error!("error sending invite message");
            return;
        }

        // create a timer for this request
        let observer = Arc::new(RequestTimerObserver {
            message: leave,
            send_slim: self.send_slim.clone(),
        });

        let timer = crate::timer::Timer::new(
            rand::random::<u32>(),
            crate::timer::TimerType::Constant,
            Duration::from_secs(1),
            None,
            Some(10),
        );
        timer.start(observer);

        self.pending_requests.insert(agent.clone(), timer);
    }

    pub async fn join(&self) {
        // subscribe for the topic
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
        let sub = Message::new_subscribe(&self.source, &self.topic, None, header);

        // TODO
        // return an error
        if self.send_slim.send(Ok(sub)).await.is_err() {
            error!("error sending invite message");
        }
    }

    pub async fn leave(&self) {
        // unsubscribe for the topic
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn));
        let sub = Message::new_unsubscribe(&self.source, &self.topic, None, header);

        // TODO
        // return an error
        if self.send_slim.send(Ok(sub)).await.is_err() {
            error!("error sending invite message");
        }
    }

    pub async fn delete_group(&mut self) {
        for a in self.group.clone() {
            self.ask_to_leave(&a).await;
        }
        self.leave().await;
    }

    pub fn process_response_msg(&mut self, msg: Message) {
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
                        self.group.insert(src);
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
                        self.group.remove(&src);
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

    fn create_request(
        &self,
        agent: &AgentType,
        agent_id: Option<u64>,
        request_type: SessionHeaderType,
    ) -> Message {
        let agp_header = Some(SlimHeader::new(&self.source, agent, agent_id, None));

        let session_header = Some(SessionHeader::new(
            request_type.into(),
            self.session_id,
            rand::random::<u32>(),
        ));

        let payload = bincode::encode_to_vec(&self.topic, bincode::config::standard()).unwrap();
        Message::new_publish_with_headers(agp_header, session_header, "", payload)
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
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let source = Agent::from_strings("org", "default", "channel", 0);
        let topic = AgentType::from_strings("org", "channel", "channel");

        let agent_to_invite = Agent::from_strings("org", "default", "to_invite", 5120);

        let mut h = ChannelModerator::new(source, topic.clone(), SESSION_ID, 1, tx_gw);

        h.invite(agent_to_invite.agent_type()).await;

        let mut request = h.create_request(
            agent_to_invite.agent_type(),
            None,
            SessionHeaderType::JoinRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        for _ in 0..11 {
            let mut msg = rx_gw.recv().await.unwrap().unwrap();
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
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let source = Agent::from_strings("org", "default", "source", 12345);
        let topic = AgentType::from_strings("org", "channel", "topic");

        let agent_to_invite = Agent::from_strings("org", "default", "to_invite", 5120);

        let mut h = ChannelModerator::new(source.clone(), topic, SESSION_ID, 1, tx_gw);

        h.invite(agent_to_invite.agent_type()).await;

        let mut request = h.create_request(
            agent_to_invite.agent_type(),
            None,
            SessionHeaderType::JoinRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        let mut msg = rx_gw.recv().await.unwrap().unwrap();
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
        h.process_response_msg(reply);

        h.ask_to_leave(&agent_to_invite).await;

        let mut request = h.create_request(
            agent_to_invite.agent_type(),
            agent_to_invite.agent_id_option(),
            SessionHeaderType::LeaveRequest,
        );
        request.get_session_header_mut().set_message_id(REQ_ID); // this is random so put a known value here

        let mut msg = rx_gw.recv().await.unwrap().unwrap();
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
        h.process_response_msg(reply);
    }
}
