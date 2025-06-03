// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::mpsc;

use crate::{
    MessageDirection, SessionMessage,
    errors::SessionError,
    producer_buffer, receiver_buffer,
    session::{
        AppChannelSender, Common, CommonSession, Id, Info, Session, SessionConfig,
        SessionConfigTrait, SessionDirection, SlimChannelSender, State,
    },
    timer,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

use slim_datapath::messages::{AgentType, utils::SlimHeaderFlags};
use slim_datapath::{
    api::proto::pubsub::v1::{Message, SessionHeader, SessionHeaderType, SlimHeader},
    messages::Agent,
};

use tonic::{Status, async_trait};
use tracing::{debug, error, trace, warn};

// this must be a number > 1
const STREAM_BROADCAST: u32 = 50;

/// Configuration for the Streaming session
#[derive(Debug, Clone, PartialEq)]
pub struct StreamingConfiguration {
    pub direction: SessionDirection,
    pub topic: AgentType,
    pub max_retries: u32,
    pub timeout: std::time::Duration,
}

impl SessionConfigTrait for StreamingConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::Streaming(config) => {
                if self.direction != config.direction {
                    return Err(SessionError::ConfigurationError(format!(
                        "cannot change session direction from {:?} to {:?}",
                        self.direction, config.direction
                    )));
                }

                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected Streaming, got {:?}",
                session_config
            ))),
        }
    }
}

impl Default for StreamingConfiguration {
    fn default() -> Self {
        StreamingConfiguration {
            direction: SessionDirection::Receiver,
            topic: AgentType::default(),
            max_retries: 10,
            timeout: std::time::Duration::from_millis(1000),
        }
    }
}

impl std::fmt::Display for StreamingConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StreamingConfiguration: max_retries: {}, timeout: {} ms",
            self.max_retries,
            self.timeout.as_millis(),
        )
    }
}

impl StreamingConfiguration {
    pub fn new(
        direction: SessionDirection,
        topic: Option<AgentType>,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        StreamingConfiguration {
            direction,
            topic: topic.unwrap_or_default(),
            max_retries: max_retries.unwrap_or(0),
            timeout: timeout.unwrap_or(std::time::Duration::from_millis(0)),
        }
    }
}

struct RtxTimerObserver {
    producer_name: Agent,
    channel: mpsc::Sender<Result<(u32, bool, Agent), Status>>,
}

#[async_trait]
impl timer::TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for rtx {}, retry", timeouts, timer_id);

        // notify the process loop
        if self
            .channel
            .send(Ok((timer_id, true, self.producer_name.clone())))
            .await
            .is_err()
        {
            error!("error notifying the process loop - rtx timer");
        }
    }

    async fn on_failure(&self, timer_id: u32, timeouts: u32) {
        trace!(
            "timeout number {} for rtx {}, stop retry",
            timeouts, timer_id
        );

        // notify the process loop
        if self
            .channel
            .send(Ok((timer_id, false, self.producer_name.clone())))
            .await
            .is_err()
        {
            error!("error notifying the process loop - rtx timer failure");
        }
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!("timer for rtx {} cancelled", timer_id);
        // nothing to do
    }
}

struct ProducerTimerObserver {
    channel: mpsc::Sender<Result<(), Status>>,
}

#[async_trait]
impl timer::TimerObserver for ProducerTimerObserver {
    async fn on_timeout(&self, _timer_id: u32, timeouts: u32) {
        trace!(
            "timeout number {} for producer timer, send beacon",
            timeouts
        );

        // notify the process loop
        if self.channel.send(Ok(())).await.is_err() {
            error!("error notifying the process loop - producer timer");
        }
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        panic!("received on failure event on producer timer",);
    }

    async fn on_stop(&self, _timer_id: u32) {
        trace!("producer timer cancelled");
        // nothing to do
    }
}

struct ProducerState {
    buffer: ProducerBuffer,
    next_id: u32,
    timer_observer: Arc<ProducerTimerObserver>,
    timer: timer::Timer,
}

struct Receiver {
    buffer: ReceiverBuffer,
    timer_observer: Arc<RtxTimerObserver>,
    rtx_map: HashMap<u32, Message>,
    timers_map: HashMap<u32, timer::Timer>,
}

struct ReceiverState {
    buffers: HashMap<Agent, Receiver>,
}

struct BidirectionalState {
    receiver: ReceiverState,
    producer: ProducerState,
}

enum Endpoint {
    Producer(ProducerState),
    Receiver(ReceiverState),
    Bidirectional(BidirectionalState),
}

pub(crate) struct Streaming {
    common: Common,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

impl Streaming {
    pub(crate) fn new(
        id: Id,
        session_config: StreamingConfiguration,
        session_direction: SessionDirection,
        agent: Agent,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
    ) -> Streaming {
        let (tx, rx) = mpsc::channel(128);
        let stream = Streaming {
            common: Common::new(
                id,
                session_direction.clone(),
                SessionConfig::Streaming(session_config.clone()),
                agent,
                tx_slim,
                tx_app,
            ),
            tx,
        };
        stream.process_message(rx, session_direction);
        stream
    }

    fn process_message(
        &self,
        mut rx: mpsc::Receiver<Result<(Message, MessageDirection), Status>>,
        session_direction: SessionDirection,
    ) {
        let session_id = self.common.id();
        let send_slim = self.common.tx_slim();
        let send_app = self.common.tx_app();
        let source = self.common.source().clone();

        let (max_retries, timeout) = match self.common.session_config() {
            SessionConfig::Streaming(streaming_configuration) => (
                streaming_configuration.max_retries,
                streaming_configuration.timeout,
            ),
            _ => {
                panic!("unable to parse streaming configuration");
            }
        };

        let (rtx_timer_tx, mut rtx_timer_rx) = mpsc::channel(128);
        let (prod_timer_tx, mut prod_timer_rx) = mpsc::channel(128);

        let mut endpoint = match session_direction {
            SessionDirection::Sender => {
                let prod = ProducerState {
                    buffer: ProducerBuffer::with_capacity(500),
                    timer_observer: Arc::new(ProducerTimerObserver {
                        channel: prod_timer_tx,
                    }),
                    timer: timer::Timer::new(
                        1,
                        timer::TimerType::Exponential,
                        Duration::from_millis(1000),
                        Some(Duration::from_secs(30)),
                        None,
                    ),
                    next_id: 0,
                };
                Endpoint::Producer(prod)
            }
            SessionDirection::Receiver => {
                let recv = ReceiverState {
                    buffers: HashMap::new(),
                };
                Endpoint::Receiver(recv)
            }
            SessionDirection::Bidirectional => {
                let producer = ProducerState {
                    buffer: ProducerBuffer::with_capacity(500),
                    timer_observer: Arc::new(ProducerTimerObserver {
                        channel: prod_timer_tx,
                    }),
                    timer: timer::Timer::new(
                        1,
                        timer::TimerType::Exponential,
                        Duration::from_millis(500),
                        Some(Duration::from_secs(30)),
                        None,
                    ),
                    next_id: 0,
                };
                let receiver = ReceiverState {
                    buffers: HashMap::new(),
                };
                let state = BidirectionalState { receiver, producer };
                Endpoint::Bidirectional(state)
            }
        };

        let mut rtx_timer_rx_closed = false;
        let mut prod_timer_rx_closed = false;

        tokio::spawn(async move {
            debug!("starting message processing on session {}", session_id);
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                debug!("no more messages to process on session {}", session_id);
                                break;
                            }
                            Some(result) => {
                                debug!("got a message in process message");
                                if result.is_err() {
                                    error!("error receiving a message on session {}, drop it", session_id);
                                    continue;
                                }
                                let (msg, direction) = result.unwrap();
                                match &mut endpoint {
                                    Endpoint::Producer(producer) => {
                                        match direction {
                                            MessageDirection::North => {
                                                trace!("received message from the gataway on producer session {}", session_id);
                                                // received a message from the SLIM
                                                // this must be an RTX message otherwise drop it
                                                match msg.get_session_header().header_type() {
                                                    SessionHeaderType::RtxRequest => {}
                                                    _ => {
                                                        error!("received invalid packet type on producer session {}: not RTX request", session_id);
                                                        continue;
                                                    }
                                                };

                                                process_incoming_rtx_request(msg, session_id, producer, &source, &send_slim).await;
                                            }
                                            MessageDirection::South => {
                                                // received a message from the APP
                                                process_message_from_app(msg, session_id, producer, false, &send_slim, &send_app).await;
                                            }
                                        }
                                    }
                                    Endpoint::Receiver(receiver) => {
                                        trace!("received message from the gataway on receiver session {}", session_id);
                                        process_message_from_slim(msg, session_id, receiver, &source, max_retries, timeout, &rtx_timer_tx, &send_slim, &send_app).await;
                                    }
                                    Endpoint::Bidirectional(state) => {
                                        match direction {
                                            MessageDirection::North => {
                                                // in this case the message can be a stream message to send to the app, or a rtx request
                                                trace!("received message from the gataway on bidirectional session {}", session_id);
                                                match msg.get_session_header().header_type() {
                                                    SessionHeaderType::RtxRequest => {
                                                        // handle RTX request
                                                        process_incoming_rtx_request(msg, session_id, &state.producer, &source, &send_slim).await;
                                                    }
                                                    _ => {
                                                        process_message_from_slim(msg, session_id, &mut state.receiver, &source, max_retries, timeout, &rtx_timer_tx, &send_slim, &send_app).await;
                                                    }
                                                }
                                            }
                                            MessageDirection::South => {
                                                // received a message from the APP
                                                process_message_from_app(msg, session_id, &mut state.producer, true, &send_slim, &send_app).await;
                                            }
                                        };
                                    }
                                }
                            }
                        }
                    }
                    next_rtx_timer = rtx_timer_rx.recv(), if !rtx_timer_rx_closed => {
                        match next_rtx_timer {
                            None => {
                                debug!("no more rtx timers to process");
                                // close the timer channel
                                rtx_timer_rx_closed = true;
                            },
                            Some(result) => {
                                if result.is_err() {
                                    error!("error receiving an RTX timer, skip it");
                                    continue;
                                }

                                let (msg_id, retry, producer_name) = result.unwrap();
                                match &mut endpoint {
                                    Endpoint::Receiver(receiver) => {
                                        if retry {
                                            handle_rtx_timeout(receiver, &producer_name, msg_id, session_id, &send_slim).await;
                                        } else {
                                            handle_rtx_failure(receiver, &producer_name, msg_id, session_id, &send_app).await;
                                        }
                                    }
                                    Endpoint::Producer(_) => {
                                        error!("received rtx timer on a producer buffer");
                                        continue;
                                    }
                                    Endpoint::Bidirectional(state) => {
                                        if retry {
                                            handle_rtx_timeout(&mut state.receiver, &producer_name, msg_id, session_id, &send_slim).await;
                                        } else {
                                            handle_rtx_failure(&mut state.receiver, &producer_name, msg_id, session_id, &send_app).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    next_prod_timer = prod_timer_rx.recv(), if !prod_timer_rx_closed => {
                        match next_prod_timer {
                            None => {
                                debug!("no more prod timers to process");
                                // close the timer channel
                                prod_timer_rx_closed = true;
                            },
                            Some(result) => match result {
                                Ok(_) => {
                                    match &mut endpoint {
                                        Endpoint::Producer(producer) => {
                                            let last_msg_id = producer.next_id - 1;
                                            debug!("received producer timer, last packet = {}", last_msg_id);

                                            send_beacon_msg(&source, producer.buffer.get_destination_name(), SessionHeaderType::BeaconStream, last_msg_id, session_id, &send_slim).await;
                                        }
                                        Endpoint::Bidirectional(state) => {
                                            let last_msg_id = state.producer.next_id - 1;
                                            debug!("received producer timer, last packet = {}", last_msg_id);

                                            send_beacon_msg(&source, state.producer.buffer.get_destination_name(), SessionHeaderType::BeaconPubSub, last_msg_id, session_id, &send_slim).await;
                                        }
                                        _ => {
                                            error!("received producer timer on a non producer buffer");
                                            continue;
                                        }
                                    }
                                }
                                Err(_) => {
                                    error!("error receiving a producer timer, skip it");
                                    continue;
                                }
                            },
                        }
                    }
                }
            }

            debug!(
                "stopping message processing on streaming session {}",
                session_id
            );
        });
    }
}

async fn process_incoming_rtx_request(
    msg: Message,
    session_id: u32,
    producer: &ProducerState,
    source: &Agent,
    send_slim: &mpsc::Sender<Result<Message, Status>>,
) {
    let msg_rtx_id = msg.get_id();

    trace!(
        "received rtx for message {} on producer session {}",
        msg_rtx_id, session_id
    );
    // search the packet in the producer buffer
    let pkt_src = msg.get_source();
    let incoming_conn = msg.get_incoming_conn();

    let rtx_pub = match producer.buffer.get(msg_rtx_id as usize) {
        Some(packet) => {
            trace!(
                "packet {} exists in the producer buffer, create rtx reply",
                msg_rtx_id
            );

            // the packet exists, send it to the source of the RTX
            let payload = match packet.get_payload() {
                Some(p) => p,
                None => {
                    error!("unable to get the payload from the packet");
                    return;
                }
            };

            let slim_header = Some(SlimHeader::new(
                source,
                pkt_src.agent_type(),
                Some(pkt_src.agent_id()),
                Some(
                    SlimHeaderFlags::default()
                        .with_forward_to(incoming_conn)
                        .with_fanout(1),
                ),
            ));

            let session_header = Some(SessionHeader::new(
                SessionHeaderType::RtxReply.into(),
                session_id,
                msg_rtx_id,
            ));

            Message::new_publish_with_headers(
                slim_header,
                session_header,
                "",
                payload.blob.to_vec(),
            )
        }
        None => {
            // the packet does not exist return an empty RtxReply with the error flag set
            debug!(
                "received an RTX messages for an old packet on session {}",
                session_id
            );

            let flags = SlimHeaderFlags::default()
                .with_forward_to(incoming_conn)
                .with_error(true);

            let slim_header = Some(SlimHeader::new(
                source,
                pkt_src.agent_type(),
                Some(pkt_src.agent_id()),
                Some(flags),
            ));

            let session_header = Some(SessionHeader::new(
                SessionHeaderType::RtxReply.into(),
                session_id,
                msg_rtx_id,
            ));

            Message::new_publish_with_headers(slim_header, session_header, "", vec![])
        }
    };

    trace!("send rtx reply for message {}", msg_rtx_id);
    if send_slim.send(Ok(rtx_pub)).await.is_err() {
        error!("error sending RTX packet to slim on session {}", session_id);
    }
}

async fn process_message_from_app(
    mut msg: Message,
    session_id: u32,
    producer: &mut ProducerState,
    is_bidirectional: bool,
    send_slim: &mpsc::Sender<Result<Message, Status>>,
    send_app: &mpsc::Sender<Result<SessionMessage, SessionError>>,
) {
    // set the session header, add the message to the buffer and send it
    trace!("received message from the app on session {}", session_id);

    if is_bidirectional {
        msg.set_header_type(SessionHeaderType::PubSub);
    } else {
        msg.set_header_type(SessionHeaderType::Stream);
    }
    msg.set_message_id(producer.next_id);
    msg.set_fanout(STREAM_BROADCAST);

    trace!(
        "add message {} to the producer buffer on session {}",
        producer.next_id, session_id
    );
    if !producer.buffer.push(msg.clone()) {
        warn!("cannot add packet to the local buffer");
    }

    trace!(
        "send message {} to the producer buffer on session {}",
        producer.next_id, session_id
    );
    producer.next_id += 1;

    if send_slim.send(Ok(msg)).await.is_err() {
        error!(
            "error sending publication packet to slim on session {}",
            session_id
        );
        send_app
            .send(Err(SessionError::Processing(
                "error sending message to local slim instance".to_string(),
            )))
            .await
            .expect("error notifying app");
    }

    // set timer for this message
    producer.timer.reset(producer.timer_observer.clone());
}

#[allow(clippy::too_many_arguments)]
async fn process_message_from_slim(
    msg: Message,
    session_id: u32,
    receiver_state: &mut ReceiverState,
    source: &Agent,
    max_retries: u32,
    timeout: Duration,
    rtx_timer_tx: &mpsc::Sender<Result<(u32, bool, Agent), Status>>,
    send_slim: &mpsc::Sender<Result<Message, Status>>,
    send_app: &mpsc::Sender<Result<SessionMessage, SessionError>>,
) {
    let producer_name = msg.get_source();
    let producer_conn = msg.get_incoming_conn();

    let receiver = match receiver_state.buffers.get_mut(&producer_name) {
        Some(state) => state,
        None => {
            let state = Receiver {
                buffer: ReceiverBuffer::default(),
                timer_observer: Arc::new(RtxTimerObserver {
                    producer_name: producer_name.clone(),
                    channel: rtx_timer_tx.clone(),
                }),
                rtx_map: HashMap::new(),
                timers_map: HashMap::new(),
            };
            // Insert the state into receiver.buffers
            receiver_state.buffers.insert(producer_name.clone(), state);
            // Return a reference to the newly inserted state
            receiver_state
                .buffers
                .get_mut(&producer_name)
                .expect("State should be present")
        }
    };

    let mut recv = Vec::new();
    let mut rtx = Vec::new();
    let header_type = msg.get_header_type();
    let msg_id = msg.get_id();

    match header_type {
        SessionHeaderType::Stream => {
            (recv, rtx) = receiver.buffer.on_received_message(msg);
        }
        SessionHeaderType::PubSub => {
            (recv, rtx) = receiver.buffer.on_received_message(msg);
        }
        SessionHeaderType::RtxReply => {
            if msg.get_error().is_some() && msg.get_error().unwrap() {
                recv = receiver.buffer.on_lost_message(msg_id);
            } else {
                (recv, rtx) = receiver.buffer.on_received_message(msg);
            }

            // try to clean local state
            match receiver.timers_map.get_mut(&msg_id) {
                Some(timer) => {
                    timer.stop();
                    receiver.timers_map.remove(&msg_id);
                    receiver.rtx_map.remove(&msg_id);
                }
                None => {
                    warn!("unable to find the timer associated to the received RTX reply");
                    // try to remove the packet anyway
                    receiver.rtx_map.remove(&msg_id);
                }
            }
        }
        SessionHeaderType::BeaconStream => {
            debug!("received stream beacon for message {}", msg_id);
            rtx = receiver.buffer.on_beacon_message(msg_id);
        }
        SessionHeaderType::BeaconPubSub => {
            debug!("received pubsub beacon for message {}", msg_id);
            rtx = receiver.buffer.on_beacon_message(msg_id);
        }
        _ => {
            error!(
                "received packet with invalid header type {} on session {}",
                i32::from(header_type),
                session_id
            );
            return;
        }
    }

    // send packets to the app
    if !recv.is_empty() {
        send_message_to_app(recv, session_id, send_app).await;
    }

    // send RTX
    for r in rtx {
        debug!(
            "packet loss detected on session {}, send RTX for id {}",
            session_id, r
        );

        let slim_header = Some(SlimHeader::new(
            source,
            producer_name.agent_type(),
            Some(producer_name.agent_id()),
            Some(SlimHeaderFlags::default().with_forward_to(producer_conn)),
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::RtxRequest.into(),
            session_id,
            r,
        ));

        let rtx = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        // set state for RTX
        let timer = timer::Timer::new(
            r,
            timer::TimerType::Constant,
            timeout,
            None,
            Some(max_retries),
        );
        timer.start(receiver.timer_observer.clone());

        receiver.rtx_map.insert(r, rtx.clone());
        receiver.timers_map.insert(r, timer);

        if send_slim.send(Ok(rtx)).await.is_err() {
            error!("error sending RTX for id {} on session {}", r, session_id);
        }
    }
}

async fn handle_rtx_timeout(
    receiver_state: &mut ReceiverState,
    producer_name: &Agent,
    msg_id: u32,
    session_id: u32,
    send_slim: &mpsc::Sender<Result<Message, Status>>,
) {
    trace!(
        "try to send rtx for packet {} on receiver session {}",
        msg_id, session_id
    );

    let receiver = match receiver_state.buffers.get_mut(producer_name) {
        Some(r) => r,
        None => {
            error!("received a timeout, but there is no state");
            return;
        }
    };

    // send the RTX again
    let rtx = match receiver.rtx_map.get(&msg_id) {
        Some(rtx) => rtx,
        None => {
            error!(
                "rtx message does not exist in the map, skip retransmission and try to stop the timer"
            );
            let timer = match receiver.timers_map.get_mut(&msg_id) {
                Some(t) => t,
                None => {
                    error!("timer not found");
                    return;
                }
            };
            timer.stop();
            return;
        }
    };

    if send_slim.send(Ok(rtx.clone())).await.is_err() {
        error!(
            "error sending RTX for id {} on session {}",
            msg_id, session_id
        );
    }
}

async fn handle_rtx_failure(
    receiver_state: &mut ReceiverState,
    producer_name: &Agent,
    msg_id: u32,
    session_id: u32,
    send_app: &mpsc::Sender<Result<SessionMessage, SessionError>>,
) {
    trace!("packet {} lost, not retries left", msg_id);

    let receiver = match receiver_state.buffers.get_mut(producer_name) {
        Some(r) => r,
        None => {
            error!("received a timeout, but there is no state");
            return;
        }
    };

    receiver.rtx_map.remove(&msg_id);
    receiver.timers_map.remove(&msg_id);

    send_message_to_app(
        receiver.buffer.on_lost_message(msg_id),
        session_id,
        send_app,
    )
    .await;
}

async fn send_beacon_msg(
    source: &Agent,
    topic: &AgentType,
    beacon_type: SessionHeaderType,
    last_msg_id: u32,
    session_id: u32,
    send_slim: &mpsc::Sender<Result<Message, Status>>,
) {
    let slim_header = Some(SlimHeader::new(
        source,
        topic,
        None,
        Some(SlimHeaderFlags::default().with_fanout(STREAM_BROADCAST)),
    ));

    let session_header = Some(SessionHeader::new(
        beacon_type.into(),
        session_id,
        last_msg_id,
    ));

    let msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

    trace!("beacon to send {:?}", msg);

    if send_slim.send(Ok(msg)).await.is_err() {
        error!("error sending beacon msg to slim on session {}", session_id);
    }
}

async fn send_message_to_app(
    messages: Vec<Option<Message>>,
    session_id: u32,
    send_app: &mpsc::Sender<Result<SessionMessage, SessionError>>,
) {
    for opt in messages {
        match opt {
            Some(m) => {
                let info = Info::from(&m);
                let session_msg = SessionMessage::new(m, info);
                // send message to the app
                if send_app.send(Ok(session_msg)).await.is_err() {
                    error!("error sending packet to slim on session {}", session_id);
                }
            }
            None => {
                warn!("a message was definitely lost in session {}", session_id);
                let _ = send_app
                    .send(Err(SessionError::MessageLost(session_id.to_string())))
                    .await;
            }
        }
    }
}

#[async_trait]
impl Session for Streaming {
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx
            .send(Ok((message.message, direction)))
            .await
            .map_err(|e| SessionError::Processing(e.to_string()))
    }
}

delegate_common_behavior!(Streaming, common);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use tokio::time;
    use tracing_test::traced_test;

    use slim_datapath::messages::AgentType;

    #[tokio::test]
    #[traced_test]
    async fn test_stream_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session_config: StreamingConfiguration =
            StreamingConfiguration::new(SessionDirection::Sender, None, None, None);

        let session = Streaming::new(
            0,
            session_config.clone(),
            SessionDirection::Sender,
            source.clone(),
            tx_slim.clone(),
            tx_app.clone(),
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Streaming(session_config.clone())
        );

        let session_config: StreamingConfiguration = StreamingConfiguration::new(
            SessionDirection::Receiver,
            None,
            Some(10),
            Some(Duration::from_millis(1000)),
        );

        let session = Streaming::new(
            1,
            session_config.clone(),
            SessionDirection::Receiver,
            source.clone(),
            tx_slim,
            tx_app,
        );

        assert_eq!(session.id(), 1);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Streaming(session_config)
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_sender_and_receiver() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_slim_receiver, _rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let session_config_sender: StreamingConfiguration =
            StreamingConfiguration::new(SessionDirection::Sender, None, None, None);
        let session_config_receiver: StreamingConfiguration = StreamingConfiguration::new(
            SessionDirection::Receiver,
            None,
            Some(5),
            Some(Duration::from_millis(500)),
        );

        let sender = Streaming::new(
            0,
            session_config_sender,
            SessionDirection::Sender,
            Agent::from_strings("cisco", "default", "sender", 0),
            tx_slim_sender,
            tx_app_sender,
        );
        let receiver = Streaming::new(
            0,
            session_config_receiver,
            SessionDirection::Receiver,
            Agent::from_strings("cisco", "default", "receiver", 0),
            tx_slim_receiver,
            tx_app_receiver,
        );

        let mut message = Message::new_publish(
            &Agent::from_strings("cisco", "default", "sender", 0),
            &AgentType::from_strings("cisco", "default", "receiver"),
            Some(0),
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;

        // set session header type for test check
        let mut expected_msg = message.clone();
        expected_msg.set_header_type(SessionHeaderType::Stream);
        expected_msg.set_fanout(STREAM_BROADCAST);

        let session_msg = SessionMessage::new(message.clone(), Info::new(0));

        // send a message from the sender app to the slim
        let res = sender
            .on_message(session_msg.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        let msg = rx_slim_sender.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);

        let session_msg = SessionMessage::new(msg, Info::new(0));
        // send the same message to the receiver
        let res = receiver
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        assert_eq!(msg.message, expected_msg);
        assert_eq!(msg.info.id, 0);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_rtx_timeouts() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_config: StreamingConfiguration = StreamingConfiguration::new(
            SessionDirection::Receiver,
            None,
            Some(5),
            Some(Duration::from_millis(500)),
        );

        let session = Streaming::new(
            0,
            session_config,
            SessionDirection::Receiver,
            Agent::from_strings("cisco", "default", "sender", 0),
            tx_slim,
            tx_app,
        );

        let mut message = Message::new_publish(
            &Agent::from_strings("cisco", "default", "sender", 0),
            &AgentType::from_strings("cisco", "default", "receiver"),
            Some(0),
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session type
        let header = message.get_session_header_mut();
        header.header_type = SessionHeaderType::Stream.into();

        let session_msg: SessionMessage = SessionMessage::new(message.clone(), Info::new(0));

        let res = session
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app.recv().await.unwrap().unwrap();
        assert_eq!(msg.message, session_msg.message);
        assert_eq!(msg.info.id, 0);

        // set msg id = 2 this will trigger a loss detection
        let header = message.get_session_header_mut();
        header.message_id = 2;

        let session_msg = SessionMessage::new(message.clone(), Info::new(0));
        let res = session
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // read rtxs from the slim channel, the original one + 5 retries
        for _ in 0..6 {
            let rtx_msg = rx_slim.recv().await.unwrap().unwrap();
            let rtx_header = rtx_msg.get_session_header();
            assert_eq!(rtx_header.session_id, 0);
            assert_eq!(rtx_header.message_id, 1);
            assert_eq!(
                rtx_header.header_type,
                i32::from(SessionHeaderType::RtxRequest)
            );
        }

        time::sleep(Duration::from_millis(1000)).await;

        let expected_msg = "packet 1 lost, not retries left";
        assert!(logs_contain(expected_msg));
        let expected_msg = "a message was definitely lost in session 0";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_rtx_reception() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(8);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(8);

        let session_config: StreamingConfiguration = StreamingConfiguration::new(
            SessionDirection::Receiver,
            None,
            Some(5),
            Some(Duration::from_millis(500)),
        );

        let session = Streaming::new(
            120,
            session_config,
            SessionDirection::Sender,
            Agent::from_strings("cisco", "default", "receiver", 0),
            tx_slim,
            tx_app,
        );

        let mut message = Message::new_publish(
            &Agent::from_strings("cisco", "default", "sender", 0),
            &AgentType::from_strings("cisco", "default", "receiver"),
            Some(0),
            None,
            "",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 120;

        let session_msg: SessionMessage = SessionMessage::new(message.clone(), Info::new(120));

        // send 3 messages
        for _ in 0..3 {
            let res = session
                .on_message(session_msg.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the slim channel
        for i in 0..3 {
            let msg = rx_slim.recv().await.unwrap().unwrap();
            let msg_header = msg.get_session_header();
            assert_eq!(msg_header.session_id, 120);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(msg_header.header_type, i32::from(SessionHeaderType::Stream));
        }

        let slim_header = Some(SlimHeader::new(
            &Agent::from_strings("cisco", "default", "sender", 0),
            &AgentType::from_strings("cisco", "default", "receiver"),
            Some(0),
            Some(
                SlimHeaderFlags::default()
                    .with_forward_to(0)
                    .with_incoming_conn(123),
            ), // set incoming conn, as it is required for the rtx
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::RtxRequest.into(),
            1,
            2,
        ));

        // receive an RTX for message 2
        let rtx = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        let session_msg: SessionMessage = SessionMessage::new(rtx.clone(), Info::new(120));

        // send the RTX from the slim
        let res = session
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // get rtx reply message from slim
        let msg = rx_slim.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 120);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(
            msg_header.header_type,
            i32::from(SessionHeaderType::RtxReply)
        );
        assert_eq!(msg.get_payload().unwrap().blob, vec![0x1, 0x2, 0x3, 0x4]);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_e2e_with_losses() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let session_config_sender: StreamingConfiguration =
            StreamingConfiguration::new(SessionDirection::Sender, None, None, None);
        let session_config_receiver: StreamingConfiguration = StreamingConfiguration::new(
            SessionDirection::Receiver,
            None,
            Some(5),
            Some(Duration::from_millis(100)), // keep the timer shorter with respect to the beacon one
                                              // otherwise we don't know which message will be received first
        );

        let sender = Streaming::new(
            0,
            session_config_sender,
            SessionDirection::Sender,
            Agent::from_strings("cisco", "default", "sender", 0),
            tx_slim_sender,
            tx_app_sender,
        );
        let receiver = Streaming::new(
            0,
            session_config_receiver,
            SessionDirection::Receiver,
            Agent::from_strings("cisco", "default", "receiver", 0),
            tx_slim_receiver,
            tx_app_receiver,
        );

        let mut message = Message::new_publish(
            &Agent::from_strings("cisco", "default", "sender", 0),
            &AgentType::from_strings("cisco", "default", "receiver"),
            Some(0),
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );
        message.set_incoming_conn(Some(0));

        let session_msg: SessionMessage = SessionMessage::new(message.clone(), Info::new(0));
        // send 3 messages from the producer app
        // send 3 messages
        for _ in 0..3 {
            let res = sender
                .on_message(session_msg.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the sender slim channel
        // forward message 1 and 3 to the receiver
        for i in 0..3 {
            let mut msg = rx_slim_sender.recv().await.unwrap().unwrap();
            let msg_header = msg.get_session_header();
            assert_eq!(msg_header.session_id, 0);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(msg_header.header_type, i32::from(SessionHeaderType::Stream));

            // the receiver should detect a loss for packet 1
            if i != 1 {
                // make sure to set the incoming connection to avoid paninc
                msg.set_incoming_conn(Some(0));
                let session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
                let res = receiver
                    .on_message(session_msg.clone(), MessageDirection::North)
                    .await;
                assert!(res.is_ok());
            }
        }

        // the receiver app should get the packet 0
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.message.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 0);
        assert_eq!(msg_header.header_type, i32::from(SessionHeaderType::Stream));
        assert_eq!(
            msg.message.get_source(),
            Agent::from_strings("cisco", "default", "sender", 0)
        );
        assert_eq!(
            msg.message.get_name(),
            (
                AgentType::from_strings("cisco", "default", "receiver"),
                Some(0)
            )
        );

        // get the RTX from packet 1 and drop the first one before send it to sender
        let msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.header_type,
            i32::from(SessionHeaderType::RtxRequest)
        );
        assert_eq!(
            msg.get_source(),
            Agent::from_strings("cisco", "default", "receiver", 0)
        );
        assert_eq!(
            msg.get_name(),
            (
                AgentType::from_strings("cisco", "default", "sender"),
                Some(0)
            )
        );

        let msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.header_type,
            i32::from(SessionHeaderType::RtxRequest)
        );
        assert_eq!(
            msg.get_source(),
            Agent::from_strings("cisco", "default", "receiver", 0)
        );
        assert_eq!(
            msg.get_name(),
            (
                AgentType::from_strings("cisco", "default", "sender"),
                Some(0)
            )
        );

        // send the second reply to the producer
        let mut session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
        // make sure to set the incoming connection to avoid paninc
        session_msg.message.set_incoming_conn(Some(0));
        let res = sender
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // this should generate an RTX reply
        let msg = rx_slim_sender.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.header_type,
            i32::from(SessionHeaderType::RtxReply)
        );
        assert_eq!(
            msg.get_source(),
            Agent::from_strings("cisco", "default", "sender", 0)
        );
        assert_eq!(
            msg.get_name(),
            (
                AgentType::from_strings("cisco", "default", "receiver"),
                Some(0)
            )
        );

        let mut session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
        // make sure to set the incoming connection to avoid paninc
        session_msg.message.set_incoming_conn(Some(0));
        let res = receiver
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the receiver app should get the packet 1 and 2, packet 1 is an RTX
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.message.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.header_type,
            i32::from(SessionHeaderType::RtxReply)
        );
        assert_eq!(
            msg.message.get_source(),
            Agent::from_strings("cisco", "default", "sender", 0)
        );
        assert_eq!(
            msg.message.get_name(),
            (
                AgentType::from_strings("cisco", "default", "receiver"),
                Some(0)
            )
        );

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.message.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(msg_header.header_type, i32::from(SessionHeaderType::Stream));
        assert_eq!(
            msg.message.get_source(),
            Agent::from_strings("cisco", "default", "sender", 0)
        );
        assert_eq!(
            msg.message.get_name(),
            (
                AgentType::from_strings("cisco", "default", "receiver"),
                Some(0)
            )
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let session_config: StreamingConfiguration =
            StreamingConfiguration::new(SessionDirection::Sender, None, None, None);

        {
            let _session = Streaming::new(
                0,
                session_config.clone(),
                SessionDirection::Sender,
                source.clone(),
                tx_slim.clone(),
                tx_app.clone(),
            );
        }

        // session should be deleted, make sure the process loop is also closed
        time::sleep(Duration::from_millis(100)).await;

        // check that the session is deleted, by checking the log
        assert!(logs_contain(
            "stopping message processing on streaming session 0"
        ));
    }
}
