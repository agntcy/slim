// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc, time::Duration};

// Third-party crates
use async_trait::async_trait;
use tokio::{
    sync::mpsc,
    time::{self, Instant},
};
use tracing::{debug, error, trace, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    Status,
    api::{
        ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType, SessionHeader,
        SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    Common, CommonSession, Id, MessageDirection, MessageHandler, SessionConfig, SessionConfigTrait,
    State, Transmitter,
    channel_endpoint::{
        ChannelEndpoint, ChannelModerator, ChannelParticipant, MlsEndpoint, MlsState,
    },
    errors::SessionError,
    producer_buffer, receiver_buffer,
    session_layer::SessionLayerMessage,
    timer,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

// this must be a number > 1
const STREAM_BROADCAST: u32 = 50;

/// Configuration for the Multicast session
#[derive(Debug, Clone, PartialEq)]
pub struct MulticastConfiguration {
    pub channel_name: Name,
    pub max_retries: u32,
    pub timeout: std::time::Duration,
    pub mls_enabled: bool,
    pub(crate) initiator: bool,
    pub metadata: HashMap<String, String>,
}

impl SessionConfigTrait for MulticastConfiguration {
    fn replace(&mut self, session_config: &SessionConfig) -> Result<(), SessionError> {
        match session_config {
            SessionConfig::Multicast(config) => {
                *self = config.clone();
                Ok(())
            }
            _ => Err(SessionError::ConfigurationError(format!(
                "invalid session config type: expected Multicast, got {:?}",
                session_config
            ))),
        }
    }
}

impl Default for MulticastConfiguration {
    fn default() -> Self {
        MulticastConfiguration {
            channel_name: Name::from_strings(["agntcy", "ns", "multicast"]),
            max_retries: 10,
            timeout: std::time::Duration::from_millis(1000),
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        }
    }
}

impl std::fmt::Display for MulticastConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MulticastConfiguration: channel_name: {}, initiator: {}, max_retries: {}, timeout: {} ms",
            self.channel_name,
            self.initiator,
            self.max_retries,
            self.timeout.as_millis(),
        )
    }
}

impl MulticastConfiguration {
    pub fn new(
        channel_name: Name,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
        mls_enabled: bool,
        metadata: HashMap<String, String>,
    ) -> Self {
        MulticastConfiguration {
            channel_name,
            max_retries: max_retries.unwrap_or(0),
            timeout: timeout.unwrap_or(std::time::Duration::from_millis(0)),
            mls_enabled,
            initiator: true,
            metadata,
        }
    }

    #[cfg(test)]
    pub fn new_with_initiator(
        channel_name: Name,
        max_retries: Option<u32>,
        timeout: Option<std::time::Duration>,
        mls_enabled: bool,
        initiator: bool,
        metadata: HashMap<String, String>,
    ) -> Self {
        Self {
            channel_name,
            max_retries: max_retries.unwrap_or(0),
            timeout: timeout.unwrap_or(std::time::Duration::from_millis(0)),
            mls_enabled,
            initiator,
            metadata,
        }
    }
}

struct RtxTimerObserver {
    producer_name: Name,
    channel: mpsc::Sender<Result<(u32, bool, Name), Status>>,
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
    buffers: HashMap<Name, Receiver>,
}

#[derive(Debug)]
pub(crate) struct Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    common: Common<P, V, T>,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

impl<P, V, T> Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Id,
        session_config: MulticastConfiguration,
        name: Name,
        tx_slim_app: T,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx_session: tokio::sync::mpsc::Sender<Result<SessionLayerMessage, SessionError>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);

        let common = Common::new(
            id,
            SessionConfig::Multicast(session_config.clone()),
            name.clone(),
            tx_slim_app.clone(),
            identity_provider,
            identity_verifier,
            session_config.mls_enabled,
            storage_path,
        );

        common.set_dst(session_config.channel_name);

        let session = Multicast { common, tx };
        session.process_message(rx, tx_session);
        session
    }

    fn process_message(
        &self,
        mut rx: mpsc::Receiver<Result<(Message, MessageDirection), Status>>,
        tx_session: tokio::sync::mpsc::Sender<Result<SessionLayerMessage, SessionError>>,
    ) {
        let session_id = self.common.id();

        let (max_retries, timeout) = match self.common.session_config() {
            SessionConfig::Multicast(multicast_configuration) => (
                multicast_configuration.max_retries,
                multicast_configuration.timeout,
            ),
            _ => {
                panic!("unable to parse multicast configuration");
            }
        };

        let (rtx_timer_tx, mut rtx_timer_rx) = mpsc::channel(128);
        let (prod_timer_tx, mut prod_timer_rx) = mpsc::channel(128);

        let mut producer = ProducerState {
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

        let mut receiver = ReceiverState {
            buffers: HashMap::new(),
        };

        let mut rtx_timer_rx_closed = false;
        let mut prod_timer_rx_closed = false;

        // get session config
        let session_config = match self.common.session_config() {
            SessionConfig::Multicast(config) => config,
            _ => {
                // this shohuld never happen
                unreachable!("invalid session config type: expected Multicast");
            }
        };

        let mls = self.common.mls();
        let tx = self.common.tx();
        let source = self.common.source().clone();
        let id = self.common.id();
        tokio::spawn(async move {
            debug!("starting message processing on session {}", session_id);

            let mls = mls.map(|mls| MlsState::new(mls).expect("failed to create MLS state"));

            let mls_enable = mls.is_some();

            // used to trigger mls key rotation
            let sleep = time::sleep(Duration::from_secs(3600));
            tokio::pin!(sleep);

            // used to send all the messages after the mls is setup
            let mut flushed = false;
            if !mls_enable {
                flushed = true;
            }

            // create the channel endpoint
            let mut channel_endpoint = match session_config.initiator {
                true => {
                    let cm = ChannelModerator::new(
                        source.clone(),
                        session_config.channel_name.clone(),
                        id,
                        ProtoSessionType::SessionMulticast,
                        60,
                        Duration::from_secs(1),
                        mls,
                        tx.clone(),
                        Some(tx_session),
                        session_config.metadata.clone(),
                    );
                    ChannelEndpoint::ChannelModerator(cm)
                }
                false => {
                    let cp = ChannelParticipant::new(
                        source.clone(),
                        session_config.channel_name.clone(),
                        id,
                        ProtoSessionType::SessionMulticast,
                        60,
                        Duration::from_secs(1),
                        mls,
                        tx.clone(),
                        session_config.metadata.clone(),
                    );
                    ChannelEndpoint::ChannelParticipant(cp)
                }
            };

            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                break;
                            }
                            Some(result) => {
                                if result.is_err() {
                                    error!(%session_id, "error receiving a message on session, drop it");
                                    continue;
                                }
                                let (msg, direction) = result.unwrap();

                                tracing::trace!("received message {} {}", msg.get_session_message_type().as_str_name(), msg.get_id());


                                // process the messages for the channel endpoint first
                                match msg.get_session_header().session_message_type() {
                                    ProtoSessionMessageType::ChannelLeaveReply => {
                                        // we need to remove the partipicant that was removed from the channel
                                        // also in the list of receiver buffers. the name to search is the
                                        // surce of the ChannelLeaveReply message
                                        let name = msg.get_source();
                                        // try to clean up the receiver buffers
                                        receiver.buffers.remove(&name);

                                        match channel_endpoint.on_message(msg).await {
                                            Ok(_) => {},
                                            Err(e) => {
                                                error!("error processing channel message: {}", e);
                                            },
                                        }
                                        continue;
                                    }
                                    ProtoSessionMessageType::ChannelDiscoveryRequest |
                                    ProtoSessionMessageType::ChannelDiscoveryReply |
                                    ProtoSessionMessageType::ChannelJoinRequest |
                                    ProtoSessionMessageType::ChannelJoinReply |
                                    ProtoSessionMessageType::ChannelLeaveRequest |
                                    ProtoSessionMessageType::ChannelMlsWelcome |
                                    ProtoSessionMessageType::ChannelMlsCommit |
                                    ProtoSessionMessageType::ChannelMlsProposal |
                                    ProtoSessionMessageType::ChannelMlsAck => {
                                        match channel_endpoint.on_message(msg).await {
                                            Ok(_) => {},
                                            Err(e) => {
                                                error!("error processing channel message: {}", e);
                                            },
                                        }

                                        // here the mls state may change, check if is it possible
                                        // to flush the producer buffer
                                        if !flushed && channel_endpoint.is_mls_up().unwrap_or(false) {
                                            // flush the producer buffer
                                            flush_producer_buffer(&mut producer, session_id, &tx).await;
                                            flushed = true;
                                        }

                                        continue;
                                    }
                                    _ => {}
                                }

                                match direction {
                                    MessageDirection::North => {
                                        // in this case the message can be message to send to the app, a rtx request,
                                        // or a channel control message to handle in the channel endpoint
                                        trace!("received message from SLIM on multicast session {}", session_id);
                                        match msg.get_session_header().session_message_type() {
                                            ProtoSessionMessageType::RtxRequest => {
                                                // handle RTX request
                                                process_incoming_rtx_request(msg, session_id, &producer, &source, &tx).await;
                                            }
                                            _ => {
                                                process_message_from_slim(msg, session_id, &mut receiver, &source, max_retries, timeout, &rtx_timer_tx, &tx).await;
                                            }
                                        }
                                    }
                                    MessageDirection::South => {
                                        // received a message from the APP
                                        // if flushed is true send the packet, otherwise keep it in the buffer
                                        process_message_from_app(msg, session_id, &mut producer, flushed, &tx).await;
                                    }
                                };

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
                                if retry {
                                    handle_rtx_timeout(&mut receiver, &producer_name, msg_id, session_id, &tx).await;
                                } else {
                                    handle_rtx_failure(&mut receiver, &producer_name, msg_id, session_id, &tx).await;
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
                                    let last_msg_id = producer.next_id - 1;
                                    debug!("received producer timer, last packet = {}", last_msg_id);

                                    send_beacon_msg(&source, producer.buffer.get_destination_name(), ProtoSessionMessageType::BeaconMulticast, last_msg_id, session_id, &tx).await;
                                }
                                Err(_) => {
                                    error!("error receiving a producer timer, skip it");
                                    continue;
                                }
                            },
                        }
                    }
                    () = &mut sleep, if mls_enable => {
                        let _ = channel_endpoint.update_mls_keys().await;
                        sleep.as_mut().reset(Instant::now() + Duration::from_secs(3600));
                    }
                }
            }

            channel_endpoint.close();

            debug!(
                "stopping message processing on multicast session {} for {}",
                session_id, source
            );
        });
    }

    pub fn with_dst<R>(&self, f: impl FnOnce(Option<&Name>) -> R) -> R {
        self.common.with_dst(f)
    }
}

async fn process_incoming_rtx_request<T>(
    msg: Message,
    session_id: u32,
    producer: &ProducerState,
    source: &Name,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
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
                &pkt_src,
                Some(
                    SlimHeaderFlags::default()
                        .with_forward_to(incoming_conn)
                        .with_fanout(1),
                ),
            ));

            let session_header = Some(SessionHeader::new(
                ProtoSessionType::SessionMulticast.into(),
                ProtoSessionMessageType::RtxReply.into(),
                session_id,
                msg_rtx_id,
                &packet.get_session_header().get_source(),
                &packet.get_session_header().get_destination(),
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

            let slim_header = Some(SlimHeader::new(source, &pkt_src, Some(flags)));

            // no need to set source and destiona here
            let session_header = Some(SessionHeader::new(
                ProtoSessionType::SessionMulticast.into(),
                ProtoSessionMessageType::RtxReply.into(),
                session_id,
                msg_rtx_id,
                &None,
                &None,
            ));

            Message::new_publish_with_headers(slim_header, session_header, "", vec![])
        }
    };

    trace!("send rtx reply for message {}", msg_rtx_id);
    if tx.send_to_slim(Ok(rtx_pub)).await.is_err() {
        error!("error sending RTX packet to slim on session {}", session_id);
    }
}

async fn flush_producer_buffer<T>(producer: &mut ProducerState, session_id: u32, tx: &T)
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    debug!("flush producer buffer");
    // flush the prod buffer and check if at least one message was sent
    let mut sent = false;
    for m in producer.buffer.iter() {
        if tx.send_to_slim(Ok(m.clone())).await.is_err() {
            error!(
                "error sending publication packet to slim on session {}",
                session_id
            );
            tx.send_to_app(Err(SessionError::Processing(
                "error sending message to local slim instance".to_string(),
            )))
            .await
            .expect("error notifying app");
        }
        sent = true;
    }

    // set timer for these messages
    if sent {
        producer.timer.reset(producer.timer_observer.clone());
    }
}

async fn process_message_from_app<T>(
    mut msg: Message,
    session_id: u32,
    producer: &mut ProducerState,
    send_msg: bool,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    // set the session header, add the message to the buffer and send it
    trace!("received message from the app on session {}", session_id);

    msg.set_session_type(ProtoSessionType::SessionMulticast);
    msg.set_session_message_type(ProtoSessionMessageType::MulticastMsg);
    msg.get_session_header_mut().set_session_id(session_id);

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

    if send_msg {
        if tx.send_to_slim(Ok(msg)).await.is_err() {
            error!(
                "error sending publication packet to slim on session {}",
                session_id
            );

            // no need to notify app here, the session will take care of retransmissions
        }

        // set timer for this message
        producer.timer.reset(producer.timer_observer.clone());
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_message_from_slim<T>(
    msg: Message,
    session_id: u32,
    receiver_state: &mut ReceiverState,
    source: &Name,
    max_retries: u32,
    timeout: Duration,
    rtx_timer_tx: &mpsc::Sender<Result<(u32, bool, Name), Status>>,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    let producer_name = msg.get_source();
    let producer_conn = msg.get_incoming_conn();

    tracing::trace!(
        "received message from SLIM {} {}",
        msg.get_session_message_type().as_str_name(),
        msg.get_id()
    );

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
    let header_type = msg.get_session_message_type();
    let msg_id = msg.get_id();

    match header_type {
        ProtoSessionMessageType::MulticastMsg => {
            (recv, rtx) = receiver.buffer.on_received_message(msg);
        }
        ProtoSessionMessageType::RtxReply => {
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
        ProtoSessionMessageType::BeaconMulticast => {
            debug!("received multicast beacon for message {}", msg_id);
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
        send_message_to_app(recv, session_id, tx).await;
    }

    // send RTX
    for r in rtx {
        debug!(
            "packet loss detected on session {}, send RTX for id {}",
            session_id, r
        );

        let slim_header = Some(SlimHeader::new(
            source,
            &producer_name,
            Some(SlimHeaderFlags::default().with_forward_to(producer_conn)),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::SessionMulticast.into(),
            ProtoSessionMessageType::RtxRequest.into(),
            session_id,
            r,
            &None,
            &None,
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

        if tx.send_to_slim(Ok(rtx)).await.is_err() {
            error!("error sending RTX for id {} on session {}", r, session_id);
        }
    }
}

async fn handle_rtx_timeout<T>(
    receiver_state: &mut ReceiverState,
    producer_name: &Name,
    msg_id: u32,
    session_id: u32,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
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

    if tx.send_to_slim(Ok(rtx.clone())).await.is_err() {
        error!(
            "error sending RTX for id {} on session {}",
            msg_id, session_id
        );
    }
}

async fn handle_rtx_failure<T>(
    receiver_state: &mut ReceiverState,
    producer_name: &Name,
    msg_id: u32,
    session_id: u32,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
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

    send_message_to_app(receiver.buffer.on_lost_message(msg_id), session_id, tx).await;
}

async fn send_beacon_msg<T>(
    source: &Name,
    topic: &Name,
    beacon_type: ProtoSessionMessageType,
    last_msg_id: u32,
    session_id: u32,
    tx: &T,
) where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    let slim_header = Some(SlimHeader::new(
        source,
        topic,
        Some(SlimHeaderFlags::default().with_fanout(STREAM_BROADCAST)),
    ));

    let session_header = Some(SessionHeader::new(
        ProtoSessionType::SessionMulticast.into(),
        beacon_type.into(),
        session_id,
        last_msg_id,
        &None,
        &None,
    ));

    let msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

    trace!("beacon to send {:?}", msg);

    if tx.send_to_slim(Ok(msg)).await.is_err() {
        error!("error sending beacon msg to slim on session {}", session_id);
    }
}

async fn send_message_to_app<T>(messages: Vec<Option<Message>>, session_id: u32, tx: &T)
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    for opt in messages {
        match opt {
            Some(m) => {
                // send message to the app
                if tx.send_to_app(Ok(m)).await.is_err() {
                    error!("error sending packet to app on session {}", session_id);
                }
            }
            None => {
                warn!("a message was definitely lost in session {}", session_id);
                let _ = tx
                    .send_to_app(Err(SessionError::MessageLost(session_id.to_string())))
                    .await;
            }
        }
    }
}

#[async_trait]
impl<P, V, T> MessageHandler for Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx.try_send(Ok((message, direction))).map_err(|e| {
            SessionError::QueueFullError(format!("session={}, error={}", self.common.id(), e))
        })
    }
}

#[async_trait]
impl<P, V, T> CommonSession<P, V, T> for Multicast<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn id(&self) -> Id {
        // concat the token stream
        self.common.id()
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_config(&self) -> SessionConfig {
        self.common.session_config()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        self.common.set_session_config(session_config)
    }

    fn source(&self) -> &Name {
        self.common.source()
    }

    fn dst(&self) -> Option<Name> {
        // multicast sessions do not use dst
        self.common.dst()
    }

    fn dst_arc(&self) -> Arc<RwLock<Option<Name>>> {
        self.common.dst_arc()
    }

    fn identity_provider(&self) -> P {
        self.common.identity_provider().clone()
    }

    fn identity_verifier(&self) -> V {
        self.common.identity_verifier().clone()
    }

    fn tx(&self) -> T {
        self.common.tx().clone()
    }

    fn tx_ref(&self) -> &T {
        self.common.tx_ref()
    }

    fn set_dst(&self, dst: Name) {
        // allow setting on the common even if unused
        self.common.set_dst(dst)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use tokio::time;
    use tracing_test::traced_test;

    use slim_datapath::messages::Name;

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_create() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter {
            slim_tx: tx_slim.clone(),
            app_tx: tx_app.clone(),
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let source = Name::from_strings(["agntcy", "ns", "local"]).with_id(0);
        let stream = Name::from_strings(["agntcy", "ns", "local_stream"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream.clone(),
            None,
            None,
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            0,
            session_config.clone(),
            source.clone(),
            tx.clone(),
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Multicast(session_config.clone())
        );

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream,
            Some(10),
            Some(Duration::from_millis(1000)),
            false,
            false,
            HashMap::new(),
        );

        let session = Multicast::new(
            1,
            session_config.clone(),
            source.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        assert_eq!(session.id(), 1);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Multicast(session_config)
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_sender_and_receiver() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::unbounded_channel();

        let tx_sender = SessionTransmitter {
            slim_tx: tx_slim_sender,
            app_tx: tx_app_sender,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let (tx_slim_receiver, _rx_slim_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::unbounded_channel();

        let tx_receiver = SessionTransmitter {
            slim_tx: tx_slim_receiver,
            app_tx: tx_app_receiver,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let send = Name::from_strings(["cisco", "default", "sender"]).with_id(0);
        let recv = Name::from_strings(["cisco", "default", "receiver"]).with_id(0);

        let session_config_sender: MulticastConfiguration =
            MulticastConfiguration::new_with_initiator(
                send.clone(),
                None,
                None,
                false,
                false,
                HashMap::new(),
            );
        let session_config_receiver: MulticastConfiguration = MulticastConfiguration::new(
            send.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let sender = Multicast::new(
            0,
            session_config_sender,
            send.clone(),
            tx_sender,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session_sender"),
            tx_session.clone(),
        );
        let receiver = Multicast::new(
            0,
            session_config_receiver,
            recv.clone(),
            tx_receiver,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session_receiver"),
            tx_session.clone(),
        );

        let mut message = Message::new_publish(
            &send,
            &recv,
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 0;

        // set session header type for test check
        let mut expected_msg = message.clone();
        expected_msg.set_session_message_type(ProtoSessionMessageType::MulticastMsg);
        expected_msg.set_session_type(ProtoSessionType::SessionMulticast);
        expected_msg.set_fanout(STREAM_BROADCAST);

        // send a message from the sender app to the slim
        let res = sender
            .on_message(message.clone(), MessageDirection::South)
            .await;
        assert!(res.is_ok());

        let msg = rx_slim_sender.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);

        // send the same message to the receiver
        let res = receiver
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);
        assert_eq!(msg.get_session_header().get_session_id(), 0);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_rtx_timeouts() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let sender = Name::from_strings(["agntcy", "ns", "sender"]).with_id(0);
        let receiver = Name::from_strings(["agntcy", "ns", "receiver"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            sender.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            0,
            session_config,
            sender.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        let mut message = Message::new_publish(
            &sender,
            &receiver,
            Some(SlimHeaderFlags::default().with_incoming_conn(123)), // set a fake incoming conn, as it is required for the rtx
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session type
        let header = message.get_session_header_mut();
        header.set_session_message_type(ProtoSessionMessageType::MulticastMsg);
        header.set_session_id(0);

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let msg = rx_app.recv().await.unwrap().unwrap();
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_session_id(), 0);

        // set msg id = 2 this will trigger a loss detection
        let header = message.get_session_header_mut();
        header.message_id = 2;

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // read rtxs from the slim channel, the original one + 5 retries
        for _ in 0..6 {
            let rtx_msg = rx_slim.recv().await.unwrap().unwrap();
            let rtx_header = rtx_msg.get_session_header();
            assert_eq!(rtx_header.session_id, 0);
            assert_eq!(rtx_header.message_id, 1);
            assert_eq!(
                rtx_header.session_message_type(),
                ProtoSessionMessageType::RtxRequest,
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
    async fn test_multicast_rtx_reception() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(8);
        let (tx_app, _rx_app) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let receiver = Name::from_strings(["agntcy", "ns", "receiver"]).with_id(0);
        let sender = Name::from_strings(["agntcy", "ns", "sender"]).with_id(0);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            sender.clone(),
            Some(5),
            Some(Duration::from_millis(500)),
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let session = Multicast::new(
            120,
            session_config,
            receiver.clone(),
            tx,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session"),
            tx_session.clone(),
        );

        let mut message =
            Message::new_publish(&sender, &receiver, None, "", vec![0x1, 0x2, 0x3, 0x4]);

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 120;

        // send 3 messages
        for _ in 0..3 {
            let res = session
                .on_message(message.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the slim channel
        for i in 0..3 {
            let msg = rx_slim.recv().await.unwrap().unwrap();
            let msg_header = msg.get_session_header();
            assert_eq!(msg_header.session_id, 120);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(
                msg_header.session_message_type(),
                ProtoSessionMessageType::MulticastMsg
            );
        }

        let slim_header = Some(SlimHeader::new(
            &sender,
            &receiver,
            Some(
                SlimHeaderFlags::default()
                    .with_forward_to(0)
                    .with_incoming_conn(123),
            ), // set incoming conn, as it is required for the rtx
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::SessionMulticast.into(),
            ProtoSessionMessageType::RtxRequest.into(),
            1,
            2,
            &None,
            &None,
        ));

        // receive an RTX for message 2
        let rtx = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        // send the RTX from the slim
        let res = session
            .on_message(rtx.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // get rtx reply message from slim
        let msg = rx_slim.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 120);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply
        );
        assert_eq!(msg.get_payload().unwrap().blob, vec![0x1, 0x2, 0x3, 0x4]);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multicast_e2e_with_losses() {
        let (tx_slim_sender, mut rx_slim_sender) = tokio::sync::mpsc::channel(3);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::unbounded_channel();

        let tx_sender = SessionTransmitter {
            slim_tx: tx_slim_sender,
            app_tx: tx_app_sender,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let (tx_slim_receiver, mut rx_slim_receiver) = tokio::sync::mpsc::channel(3);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::unbounded_channel();

        let tx_receiver = SessionTransmitter {
            slim_tx: tx_slim_receiver,
            app_tx: tx_app_receiver,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let send = Name::from_strings(["cisco", "default", "sender"]).with_id(0);
        let recv = Name::from_strings(["cisco", "default", "receiver"]).with_id(0);

        let session_config_sender: MulticastConfiguration =
            MulticastConfiguration::new_with_initiator(
                recv.clone(),
                None,
                None,
                false,
                false,
                HashMap::new(),
            );
        let session_config_receiver: MulticastConfiguration = MulticastConfiguration::new(
            recv.clone(),
            Some(5),
            Some(Duration::from_millis(100)), // keep the timer shorter with respect to the beacon one
            // otherwise we don't know which message will be received first
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        let sender = Multicast::new(
            0,
            session_config_sender,
            send.clone(),
            tx_sender,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session_sender"),
            tx_session.clone(),
        );
        let receiver = Multicast::new(
            0,
            session_config_receiver,
            recv.clone(),
            tx_receiver,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            std::path::PathBuf::from("/tmp/test_session_receiver"),
            tx_session.clone(),
        );

        let mut message = Message::new_publish(
            &send,
            &recv,
            Some(SlimHeaderFlags::default().with_incoming_conn(0)),
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );
        message.set_incoming_conn(Some(0));
        message.get_session_header_mut().set_session_id(0);

        // send 3 messages from the producer app
        // send 3 messages
        for _ in 0..3 {
            let res = sender
                .on_message(message.clone(), MessageDirection::South)
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
            assert_eq!(
                msg_header.session_message_type(),
                ProtoSessionMessageType::MulticastMsg
            );

            // the receiver should detect a loss for packet 1
            if i != 1 {
                // make sure to set the incoming connection to avoid panic
                msg.set_incoming_conn(Some(0));
                msg.get_session_header_mut().set_session_id(0);
                let res = receiver
                    .on_message(msg.clone(), MessageDirection::North)
                    .await;
                assert!(res.is_ok());
            }
        }

        // the receiver app should get the packet 0
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 0);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        // get the RTX from packet 1 and drop the first one before send it to sender
        let msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxRequest,
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );

        let mut msg = rx_slim_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxRequest
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );

        // send the second reply to the producer
        // make sure to set the incoming connection to avoid paninc
        msg.set_incoming_conn(Some(0));
        msg.get_session_header_mut().set_session_id(0);
        let res = sender
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // this should generate an RTX reply
        let mut msg = rx_slim_sender.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        // make sure to set the incoming connection to avoid paninc
        msg.set_incoming_conn(Some(0));
        let res = receiver
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the receiver app should get the packet 1 and 2, packet 1 is an RTX
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::RtxReply,
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = msg.get_session_header();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(
            msg_header.session_message_type(),
            ProtoSessionMessageType::MulticastMsg
        );
        assert_eq!(
            msg.get_source(),
            Name::from_strings(["cisco", "default", "sender"]).with_id(0)
        );
        assert_eq!(
            msg.get_dst(),
            Name::from_strings(["cisco", "default", "receiver"]).with_id(0)
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_session_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::unbounded_channel();

        let tx = SessionTransmitter {
            slim_tx: tx_slim,
            app_tx: tx_app,
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        let source = Name::from_strings(["agntcy", "ns", "local"]).with_id(0);
        let stream = Name::from_strings(["agntcy", "ns", "stream"]);

        let session_config: MulticastConfiguration = MulticastConfiguration::new_with_initiator(
            stream,
            None,
            None,
            false,
            false,
            HashMap::new(),
        );

        let (tx_session, _rx_session) = tokio::sync::mpsc::channel(16);

        {
            let _session = Multicast::new(
                0,
                session_config.clone(),
                source.clone(),
                tx,
                SharedSecret::new("a", "group"),
                SharedSecret::new("a", "group"),
                std::path::PathBuf::from("/tmp/test_session"),
                tx_session.clone(),
            );
        }

        // session should be deleted, make sure the process loop is also closed
        time::sleep(Duration::from_millis(100)).await;

        // check that the session is deleted, by checking the log
        assert!(logs_contain(
            "stopping message processing on multicast session 0"
        ));
    }
}
