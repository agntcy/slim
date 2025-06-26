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
    messages::{Agent, AgentType, utils::SlimHeaderFlags},
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
trait OnMessageReceived {
    async fn on_message(&mut self, msg: Message);
}

#[derive(Debug)]
pub enum ChannelEndpoint {
    ChannelParticipant(ChannelParticipant),
    #[allow(dead_code)]
    ChannelModerator(ChannelModerator),
}

impl ChannelEndpoint {
    pub async fn invite(&mut self, endpoint_type: &AgentType) {
        match self {
            ChannelEndpoint::ChannelParticipant(_) => {
                error!("invite cannot ne performed by a participant")
            }
            ChannelEndpoint::ChannelModerator(cm) => {
                cm.invite(endpoint_type).await;
            }
        }
    }

    pub async fn remove_endpoint(&mut self, endpoint_type: &Agent) {
        match self {
            ChannelEndpoint::ChannelParticipant(_) => {
                error!("remove cannot ne performed by a participant")
            }
            ChannelEndpoint::ChannelModerator(cm) => {
                cm.ask_to_leave(endpoint_type).await;
            }
        }
    }

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
}

impl Endpoint {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
    ) -> Self {
        Endpoint {
            name: name.clone(),
            channel_name: channel_name.clone(),
            session_id,
            conn,
            subscribed: false,
            send_slim,
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
        let agp_header = Some(SlimHeader::new(
            &self.name,
            destination,
            destination_id,
            None,
        ));

        let session_header = Some(SessionHeader::new(
            request_type.into(),
            self.session_id, // TODO. are we sure that we always know the session id?
            message_id,
            //rand::random::<u32>(),
        ));

        //let payload = bincode::encode_to_vec(&self.endpoint.channel_name, bincode::config::standard()).unwrap();
        Message::new_publish_with_headers(agp_header, session_header, "", payload)
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

#[allow(dead_code)]
#[derive(Debug)]
pub struct ChannelParticipant {
    endpoint: Endpoint,
}

#[allow(dead_code)]
impl ChannelParticipant {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType, // TODO: this may be unknown at the beginning, probably it has to be optional
        session_id: Id,
        conn: u64,
        send_slim: SlimChannelSender,
    ) -> Self {
        let endpoint = Endpoint::new(name, channel_name, session_id, conn, send_slim);
        ChannelParticipant { endpoint }
    }
}

impl OnMessageReceived for ChannelParticipant {
    async fn on_message(&mut self, msg: Message) {
        let msg_type = msg.get_session_header().header_type();
        match msg_type {
            SessionHeaderType::ChannelDiscoveryRequest => {
                debug!("received discovery request message");
                // send te name to the source of the message without setting any state
                // in this way if the message gets lost the sender can restart the proceduer
                // without any problem of dandling state
                let src = msg.get_source();
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelDiscoveryReply,
                    msg.get_id(),
                    vec![],
                );
                // TODO handle error
                self.endpoint.send(reply).await;
            }
            SessionHeaderType::ChannelJoinRequest => {
                debug!("received join request message");
                // accept the join and set the state:
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
                // TODO: handle error
                self.endpoint.join().await;

                // reply to the request
                let reply = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelJoinReply,
                    msg.get_id(),
                    vec![],
                );
                // TODO: handle error
                self.endpoint.send(reply).await;
            }
            SessionHeaderType::ChannelLeaveRequest => {
                debug!("received leave request message");
                // leave the channell
                // TODO: handle error
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

                // TODO: handle error
                self.endpoint.send(reply).await;
            }
            _ => {
                error!("received unexpected packet type");
            }
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ChannelModerator {
    endpoint: Endpoint,

    /// list of endpoint names in the channel
    channel_list: HashSet<Agent>,

    /// list of pending requests and related timers
    pending_requests: HashMap<u32, crate::timer::Timer>,

    /// number or maximum retries before give up with a control message
    max_reties: u32,

    /// interval between retries
    retries_interval: Duration,
}

#[allow(dead_code)]
impl ChannelModerator {
    pub fn new(
        name: &Agent,
        channel_name: &AgentType,
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

    pub async fn join(&mut self) {
        if !self.endpoint.subscribed {
            // TODO: handle error
            self.endpoint.join().await;

            // add the moderator to the channel
            self.channel_list.insert(self.endpoint.name.clone());
        }
    }

    pub async fn invite(&mut self, endpoint_type: &AgentType) {
        // the invite procedure works in two steps.
        // 1. discover an endpoint of the require type sending a discovery message
        // 2. invite the endpoint to the channel on the reception of the reply from the first message

        let msg_id = rand::random::<u32>();
        let discover = self.endpoint.create_channel_message(
            endpoint_type,
            None,
            SessionHeaderType::ChannelDiscoveryRequest,
            msg_id,
            vec![],
        );

        // TODO: handle error
        self.endpoint.send(discover.clone()).await;

        // create a timer for this request
        self.create_timer(msg_id, discover);
    }

    pub async fn ask_to_leave(&mut self, endpoint: &Agent) {
        let msg_id = rand::random::<u32>();
        let leave = self.endpoint.create_channel_message(
            endpoint.agent_type(),
            endpoint.agent_id_option(),
            SessionHeaderType::ChannelLeaveRequest,
            msg_id,
            vec![],
        );

        // TODO: handle error
        self.endpoint.send(leave.clone()).await;

        // create a timer for this request
        self.create_timer(msg_id, leave);
    }

    pub async fn delete_group(&mut self) {
        for a in self.channel_list.clone() {
            self.ask_to_leave(&a).await;
        }

        self.endpoint.leave().await;
    }

    fn create_timer(&mut self, key: u32, msg: Message) {
        let observer = Arc::new(RequestTimerObserver {
            message: msg,
            send_slim: self.endpoint.send_slim.clone(),
        });

        let timer = crate::timer::Timer::new(
            key,
            crate::timer::TimerType::Constant,
            self.retries_interval,
            None,
            Some(self.max_reties),
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
            SessionHeaderType::ChannelDiscoveryReply => {
                debug!("received discovery reply message");
                // an endpoint replyed to the discovery message
                // send a join message
                let src = msg.get_slim_header().get_source();
                let recv_msg_id = msg.get_id();
                let new_msg_id = rand::random::<u32>();
                // TODO: error
                let payload: Vec<u8> = bincode::encode_to_vec(
                    &self.endpoint.channel_name,
                    bincode::config::standard(),
                )
                .unwrap();

                let join = self.endpoint.create_channel_message(
                    src.agent_type(),
                    src.agent_id_option(),
                    SessionHeaderType::ChannelJoinRequest,
                    new_msg_id,
                    payload,
                );
                // TODO: handle error
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
    use tokio::time;
    use tracing_test::traced_test;

    use slim_datapath::messages::AgentType;

    const SESSION_ID: u32 = 10;

    #[tokio::test]
    #[traced_test]
    async fn test_no_endpoint_reply() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(50);
        let source = Agent::from_strings("org", "default", "source", 0);
        let endpoint = Agent::from_strings("org", "default", "endpoint", 100);
        let channel_name = AgentType::from_strings("org", "default", "channel");
        let conn = 1;

        let mut cm = ChannelModerator::new(
            &source,
            &channel_name,
            SESSION_ID,
            conn,
            3,
            Duration::from_millis(100),
            tx_slim,
        );

        cm.join().await;
        // the first message received  by slim should a subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&source, &channel_name, None, header);
        let msg = rx_slim.recv().await.unwrap().unwrap();
        assert_eq!(sub, msg);

        // test invite function
        cm.invite(endpoint.agent_type()).await;

        // create a request to compare with the output of the invite fn
        let mut request = cm.endpoint.create_channel_message(
            endpoint.agent_type(),
            None,
            SessionHeaderType::ChannelDiscoveryRequest,
            0,
            vec![],
        );

        for _ in 0..4 {
            let msg = rx_slim.recv().await.unwrap().unwrap();
            let msg_id = msg.get_id();
            request.get_session_header_mut().set_message_id(msg_id);
            assert_eq!(msg, request);
        }

        time::sleep(Duration::from_millis(1000)).await;

        // the timer will fail and print the message
        let expected_msg = "unable to send message";
        assert!(logs_contain(expected_msg));

        // test leave fn in the same way
        cm.ask_to_leave(&endpoint.clone()).await;

        // create a request to compare with the output of the invite fn
        let mut request = cm.endpoint.create_channel_message(
            endpoint.agent_type(),
            endpoint.agent_id_option(),
            SessionHeaderType::ChannelLeaveRequest,
            0,
            vec![],
        );

        for _ in 0..4 {
            let msg = rx_slim.recv().await.unwrap().unwrap();
            let msg_id = msg.get_id();
            request.get_session_header_mut().set_message_id(msg_id);
            assert_eq!(msg, request);
        }

        time::sleep(Duration::from_millis(1000)).await;

        // the timer will fail and print the message
        let expected_msg = "unable to send message";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_full_join_and_leave() {
        let (moderator_tx, mut moderator_rx) = tokio::sync::mpsc::channel(50);
        let (participant_tx, mut participant_rx) = tokio::sync::mpsc::channel(50);

        let moderator = Agent::from_strings("org", "default", "moderator", 12345);
        let participant = Agent::from_strings("org", "default", "participant", 5120);
        let channel_name = AgentType::from_strings("org", "channel", "topic");
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

        cm.join().await;
        // the first message received  by slim should a subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(sub, msg);
        assert_eq!(cm.channel_list.len(), 1);
        assert!(cm.channel_list.contains(&moderator));

        cm.invite(participant.agent_type()).await;

        // create a request to compare with the output of the invite fn
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            None,
            SessionHeaderType::ChannelDiscoveryRequest,
            0,
            vec![],
        );

        // get discovery request
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
        assert_eq!(msg, request);

        // mimic message reception on participant side
        cp.on_message(msg).await;

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            SessionHeaderType::ChannelDiscoveryReply,
            msg_id,
            vec![],
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);

        // mimic message reception on moderator side
        cm.on_message(msg).await;

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
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
        assert_eq!(msg, request);

        // mimic message reception on participant side
        cp.on_message(msg).await;

        // the first message received  by slim should a subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(sub, msg);

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            SessionHeaderType::ChannelJoinReply,
            msg_id,
            vec![],
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);

        // mimic message reception on moderator side
        cm.on_message(msg).await;

        // nothing should happen except that the new participant
        // now is in the channel list and the pending request size
        // is zero
        assert_eq!(cm.channel_list.len(), 2);
        assert!(cm.channel_list.contains(&moderator));
        assert!(cm.channel_list.contains(&participant));
        assert_eq!(cm.pending_requests.len(), 0);

        // ask to leave to the participant
        cm.ask_to_leave(&participant).await;
        // create a request to compare with the output of the invite fn
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            SessionHeaderType::ChannelLeaveRequest,
            0,
            vec![],
        );

        // get discovery request
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
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
