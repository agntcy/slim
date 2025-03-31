// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

use crate::{
    errors::SessionError,
    producer_buffer, receiver_buffer,
    session::{
        AppChannelSender, Common, CommonSession, GwChannelSender, Id, Info, Session, SessionConfig,
        SessionDirection, State,
    },
    timer::{Timer, TimerObserver},
    MessageDirection, SessionMessage,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

use agp_datapath::{
    messages::utils::{get_msg_id, get_payload_from_msg},
    pubsub::proto::pubsub::v1::SessionHeaderType,
};
use agp_datapath::{
    messages::{
        utils::{
            create_rtx_publication, get_session_header_type, get_source, service_type_to_int,
            set_msg_id, set_session_type,
        },
        Agent, AgentType,
    },
    pubsub::proto::pubsub::v1::Message,
};

use tonic::{async_trait, Status};
use tracing::{debug, error, info, trace, warn};

/// Configuration for the Streaming session
#[derive(Debug, Clone, PartialEq)]
pub struct StreamingConfiguration {
    pub max_retries: u32,
    pub timeout: std::time::Duration,
    pub source: Agent,
}

impl Default for StreamingConfiguration {
    fn default() -> Self {
        StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(1000),
            source: Agent::default(),
        }
    }
}

impl std::fmt::Display for StreamingConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StreamingConfiguration: max_retries: {}, timeout: {} ms, source: {}",
            self.max_retries,
            self.timeout.as_millis(),
            self.source,
        )
    }
}

#[allow(dead_code)]
struct RtxTimerObserver {
    channel: mpsc::Sender<Result<(u32, bool), Status>>,
}

#[async_trait]
impl TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for rtx {}, retry", timeouts, timer_id);

        // notify the process loop
        if self.channel.send(Ok((timer_id, true))).await.is_err() {
            error!("error notifying the process loop");
        }
    }

    async fn on_failure(&self, timer_id: u32, timeouts: u32) {
        trace!(
            "timeout number {} for rtx {}, stop retry",
            timeouts,
            timer_id
        );

        // notify the process loop
        if self.channel.send(Ok((timer_id, false))).await.is_err() {
            error!("error notifying the process loop");
        }
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!("timer for rtx {} cancelled", timer_id);
        // nothing to do
    }
}

#[allow(dead_code)]
struct Producer {
    buffer: ProducerBuffer,
    next_id: u32,
}

#[allow(dead_code)]
struct Receiver {
    config: StreamingConfiguration,
    buffer: ReceiverBuffer,
    timer_observer: Arc<RtxTimerObserver>,
    rtx_map: HashMap<u32, Message>,
    timers_map: HashMap<u32, Timer>,
}

#[allow(dead_code)]
enum Endpoint {
    Producer(Producer),
    Receiver(Receiver),
    Unknown,
}

#[allow(dead_code)]
pub(crate) struct Streaming {
    common: Common,
    buffer: Arc<RwLock<Endpoint>>,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

#[allow(dead_code)]
impl Streaming {
    pub(crate) fn new(
        id: Id,
        session_config: StreamingConfiguration,
        session_direction: SessionDirection,
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> Streaming {
        let (tx, rx) = mpsc::channel(128);
        let (timer_tx, timer_rx) = mpsc::channel(128);
        if session_direction == SessionDirection::Sender {
            let prod = Producer {
                buffer: ProducerBuffer::with_capacity(500),
                next_id: 0,
            };
            let stream = Streaming {
                common: Common::new(
                    id,
                    session_direction,
                    SessionConfig::Streaming(session_config),
                    tx_gw,
                    tx_app,
                ),
                buffer: Arc::new(RwLock::new(Endpoint::Producer(prod))),
                tx,
            };
            stream.process_message(rx, timer_rx);
            stream
        } else {
            let observer = RtxTimerObserver { channel: timer_tx };

            let recv = Receiver {
                config: session_config.clone(),
                buffer: ReceiverBuffer::default(),
                timer_observer: Arc::new(observer),
                rtx_map: HashMap::new(),
                timers_map: HashMap::new(),
            };

            let stream = Streaming {
                common: Common::new(
                    id,
                    session_direction,
                    SessionConfig::Streaming(session_config),
                    tx_gw,
                    tx_app,
                ),
                buffer: Arc::new(RwLock::new(Endpoint::Receiver(recv))),
                tx,
            };

            stream.process_message(rx, timer_rx);
            stream
        }
    }

    fn process_message(
        &self,
        mut rx: mpsc::Receiver<Result<(Message, MessageDirection), Status>>,
        mut timer_rx: mpsc::Receiver<Result<(u32, bool), Status>>,
    ) {
        let buffer_clone = self.buffer.clone();
        let session_id = self.common.id();
        let send_gw = self.common.tx_gw();
        let send_app = self.common.tx_app();
        //let state = self.common.state().clone();
        let mut timer_rx_closed = false;
        tokio::spawn(async move {
            debug!("starting message processing on session {}", session_id);
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                info!("no more messages to process on session {}", session_id);
                                break;
                            }
                            Some(result) => {
                                debug!("got a message in process message");
                                if result.is_err() {
                                    error!("error receiving a message on session {}, drop it", session_id);
                                    continue;
                                }
                                let (mut msg, direction) = result.unwrap();
                                match &mut *buffer_clone.write().await {
                                    Endpoint::Producer(producer) => {
                                        match direction {
                                            MessageDirection::North => {
                                                trace!("received message from the gataway on session {}", session_id);
                                                // received a message from the GW
                                                // this must be an RTX message otherwise drop it
                                                match get_session_header_type(&msg){
                                                    Ok(session_type) => {
                                                        if session_type != service_type_to_int(SessionHeaderType::RtxRequest) {
                                                            error!("received invalid packet type on session {}: not RTX request", session_id);
                                                            continue;
                                                        }
                                                        session_type
                                                    }
                                                    Err(_) => {
                                                        error!("received invalid packet type on session {}: missing header type", session_id);
                                                        continue;
                                                    }
                                                };

                                                let msg_rtx_id = match get_msg_id(&msg) {
                                                    Ok(msg_rtx_id) => msg_rtx_id,
                                                    Err(_) => {
                                                        error!("error parsing request RTX message on session {}", session_id);
                                                        continue;
                                                    }
                                                };

                                                trace!("received rtx for message {} on session {}", msg_rtx_id, session_id);
                                                // search the packet in the producer buffer
                                                let rtx_pub = match producer.buffer.get(msg_rtx_id as usize) {
                                                    Some(packet) => {
                                                        trace!("packet {} exists in the producer buffer, create rtx reply", msg_rtx_id);
                                                        // the packet exists, send it to the source of the RTX
                                                        let src = match get_source(&msg) {
                                                            Ok(src) => src,
                                                            Err(_) => {
                                                                error!("error parsing received RTX on session {}: missing source", session_id);
                                                                continue;
                                                            }
                                                        };

                                                        // TODO -> get the name of the this agent to add it as source in the RXT packet
                                                        /*let (dst_type, dst_id) = match get_source(&packet) {
                                                            Ok((dst_type, dst_id)) => (dst_type, dst_id),
                                                            Err(_) => {
                                                                error!("error parsing packet from local buffer on session {}: missing source", session_id);
                                                                continue;
                                                            }
                                                        };*/

                                                        // TODO -> get the name of the this agent to add it as source in the RXT packet
                                                        let payload = match get_payload_from_msg(&packet) {
                                                            Ok(payload) => payload,
                                                            Err(e) => {
                                                                error!("error parsing packet: {}", e.to_string());
                                                                continue;
                                                            }
                                                        };
                                                        create_rtx_publication(&Agent::default(), src.agent_type(), src.agent_id_option(), false, session_id, msg_rtx_id, Some(payload.to_vec()))
                                                    }
                                                    None => {
                                                        // the packet does not exist so do nothing
                                                        // TODO(micpapal): improve by returning a rtx nack so that the remote app does not
                                                        // wait too long for all the retransmissions
                                                        debug!("received and RTX messages for an old packet on session {}", session_id);
                                                        continue;
                                                    },
                                                };

                                                trace!("send rtx reply for message {}", msg_rtx_id);
                                                if send_gw.send(Ok(rtx_pub)).await.is_err() {
                                                    error!("error sending packet to the gateway on session {}", session_id);
                                                    continue; // TODO: do we need to notify the app with an error packet?
                                                }
                                            }
                                            MessageDirection::South => {
                                                // received a message from the APP
                                                // set the session header, add the message to the buffer and send it
                                                trace!("received message from the app on session {}", session_id);
                                                if set_session_type(&mut msg, SessionHeaderType::Stream).is_err() {
                                                    error!("error setting session type, drop packet");
                                                    continue; // TODO: do we need to notify the app with an error packet?
                                                }

                                                if set_msg_id(&mut msg, producer.next_id).is_err() {
                                                    error!("error setting msg id. drop packet");
                                                    continue; // TODO: do we need to notify the app with an error packet?
                                                }

                                                trace!("add message {} to the producer buffer on session {}", producer.next_id, session_id);
                                                if !producer.buffer.push(msg.clone()) {
                                                    warn!("cannot add packet to the local buffer");
                                                }

                                                trace!("send message {} to the producer buffer on session {}", producer.next_id, session_id);
                                                producer.next_id += 1;

                                                if send_gw.send(Ok(msg)).await.is_err() {
                                                    error!("error sending packet to the gateway on session {}", session_id);
                                                    continue; // TODO: do we need to notify the app with an error packet?
                                                }
                                            }
                                        }
                                    }
                                    Endpoint::Receiver(receiver) => {
                                        trace!("received message from the gataway on session {}, add it to the receiver buffer", session_id);
                                        let header_type = match get_session_header_type(&msg){
                                            Ok(t) => t,
                                            Err(e) => {
                                                error!("unable to parse received packet {}", e.to_string());
                                                continue;
                                            },
                                        };
                                        if header_type == service_type_to_int(SessionHeaderType::RtxReply) {
                                            let rtx_msg_id = match get_msg_id(&msg){
                                                Ok(id) => id,
                                                Err(e) => {
                                                    error!("unable to parse received packet {}", e.to_string());
                                                    continue;
                                                },
                                            };
                                            match receiver.timers_map.get(&rtx_msg_id) {
                                                Some(timer) => {
                                                    timer.stop();
                                                    receiver.timers_map.remove(&rtx_msg_id);
                                                    receiver.rtx_map.remove(&rtx_msg_id);
                                                }
                                                None => {
                                                    warn!("unable to find the timer associated to the received RTX reply");
                                                    // try to remove the packet anyway
                                                    receiver.rtx_map.remove(&rtx_msg_id);
                                                }
                                            }

                                        } else if header_type != service_type_to_int(SessionHeaderType::Stream){
                                            error!("received packet with invalid header type: {}", header_type);
                                            continue;
                                        }

                                        match receiver.buffer.on_received_message(msg){
                                            Ok((recv, rtx)) =>{
                                                for opt in recv {
                                                    trace!("send recv packet to the application on session {}", session_id);
                                                    match opt {
                                                        Some(m) => {
                                                            // send message to the app
                                                            let info = Info::from(&m);
                                                            let session_msg = SessionMessage::new(m, info);
                                                            if send_app.send(Ok(session_msg)).await.is_err() {
                                                                // TODO: How to notify the application here?
                                                                error!("error sending packet to the application on session {}", session_id);
                                                            }
                                                        }
                                                        None => {
                                                            warn!("a message was definitely lost in session {}", session_id);
                                                            let _ = send_app.send(Err(SessionError::LostMessage(session_id))).await;
                                                        }
                                                    }
                                                }
                                                for r in rtx {
                                                    debug!("send a rtx for message {} on session {}", r, session_id);
                                                    // TODO get the agent name
                                                    // TODO get the destination
                                                    let rtx = create_rtx_publication(&Agent::default(), &AgentType::default(), None, true, session_id, r, Some(vec![]));

                                                    // set state for RTX
                                                    let timer = Timer::new(r, 500, 5);
                                                    timer.start(receiver.timer_observer.clone());

                                                    receiver.rtx_map.insert(r, rtx.clone());
                                                    receiver.timers_map.insert(r, timer);

                                                    if send_gw.send(Ok(rtx)).await.is_err() {
                                                        // TODO: How to notify the application here?
                                                        error!("error sending RTX for id {} on session {}", r, session_id);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error!("error adding message to the buffer: {}", e.to_string());
                                                continue; // TODO: How to notify the application here?
                                            }
                                        }
                                    }
                                    Endpoint::Unknown => {
                                        error!("unknown session type");
                                        continue;
                                    },
                                }
                            }
                        }
                    }
                    next_timer = timer_rx.recv(), if !timer_rx_closed => {
                        match next_timer {
                            None => {
                                info!("no more timers to process");
                                // close the timer channel
                                timer_rx_closed = true;
                            },
                            Some(result) => {
                                if result.is_err() {
                                    error!("error receiving a timer, skip it");
                                    continue;
                                }

                                let (msg_id, retry) = result.unwrap();
                                match &mut *buffer_clone.write().await {
                                    Endpoint::Receiver(receiver) => {
                                        if retry {
                                            trace!("try to send rtx for packet {}", msg_id);
                                            // send the RTX again
                                            let rtx = match receiver.rtx_map.get(&msg_id) {
                                                Some(rtx) => rtx,
                                                None => {
                                                    error!("rtx message does not exist in the map, skip retransmission and try to stop the timer");
                                                    let timer = match receiver.timers_map.get(&msg_id) {
                                                        Some(t) => t,
                                                        None => {
                                                            error!("timer not found");
                                                            continue;
                                                        },
                                                    };
                                                    timer.stop();
                                                    continue;
                                                }
                                            };

                                            if send_gw.send(Ok(rtx.clone())).await.is_err() {
                                                error!("error sending RTX for id {} on session {}", msg_id, session_id);
                                            }
                                        } else {
                                            trace!("pacekt {} lost, not retry left", msg_id);
                                            receiver.rtx_map.remove(&msg_id);
                                            receiver.timers_map.remove(&msg_id);

                                            match receiver.buffer.on_lost_message(msg_id) {
                                                Ok(recv) => {
                                                    for opt in recv {
                                                        match opt {
                                                            Some(m) => {
                                                                let info = Info::from(&m);
                                                                let session_msg = SessionMessage::new(m, info);
                                                                // send message to the app
                                                                if send_app.send(Ok(session_msg)).await.is_err() {
                                                                    // TODO: How to notify the application here?
                                                                    error!("error sending packet to the gateway on session {}", session_id);
                                                                }
                                                            }
                                                            None => {
                                                                warn!("a message was definitely lost in session {}", session_id);
                                                                let _ = send_app.send(Err(SessionError::LostMessage(session_id))).await;
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    // TODO: How to notify the application here?
                                                    error!("error adding message lost to the buffer: {}", e.to_string());
                                                    continue;
                                                }
                                            };
                                        }
                                    }
                                    Endpoint::Producer(_) => {
                                        error!("received timer on a producer buffer");
                                        continue;
                                    }
                                    Endpoint::Unknown => {
                                        error!("unknown session type");
                                        continue;
                                    },
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

#[async_trait]
impl Session for Streaming {
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        debug!("receive message");
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
    use agp_datapath::messages::{encoder, utils};
    use tokio::time;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_stream_create() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_config = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "local_agent", 0),
        };

        let session = Streaming::new(
            0,
            session_config.clone(),
            SessionDirection::Sender,
            tx_gw.clone(),
            tx_app.clone(),
        );

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(
            session.session_config(),
            SessionConfig::Streaming(session_config.clone())
        );

        let session = Streaming::new(
            1,
            session_config.clone(),
            SessionDirection::Receiver,
            tx_gw,
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
        let (tx_gw_sender, mut rx_gw_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_gw_receiver, _rx_gw_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let session_config_sender = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "local_agent", 0),
        };

        let session_config_receiver = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "remote_agent", 0),
        };

        let sender = Streaming::new(
            0,
            session_config_sender,
            SessionDirection::Sender,
            tx_gw_sender,
            tx_app_sender,
        );
        let receiver = Streaming::new(
            0,
            session_config_receiver,
            SessionDirection::Receiver,
            tx_gw_receiver,
            tx_app_receiver,
        );

        let mut message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
        header.session_id = 0;

        // set session header type for test check
        let mut expected_msg = message.clone();
        let _ = set_session_type(&mut expected_msg, SessionHeaderType::Stream);

        let session_msg = SessionMessage::new(message.clone(), Info::new(0));

        // send a message from the sender app to the gw
        let res = sender
            .on_message(session_msg.clone(), MessageDirection::South) // !!!! WHAT IS THE RIGHT DIRECTION
            .await;
        assert!(res.is_ok());

        let msg = rx_gw_sender.recv().await.unwrap().unwrap();
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
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_config = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "local_agent", 0),
        };

        let session = Streaming::new(0, session_config, SessionDirection::Receiver, tx_gw, tx_app);

        let mut message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session type
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
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
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
        header.message_id = 2;

        let session_msg = SessionMessage::new(message.clone(), Info::new(0));
        let res = session
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // read rtxs from the gw channel, the original one + 5 retries
        for _ in 0..6 {
            let rtx_msg = rx_gw.recv().await.unwrap().unwrap();
            let rtx_header = utils::get_session_header(&rtx_msg).unwrap();
            assert_eq!(rtx_header.session_id, 0);
            assert_eq!(rtx_header.message_id, 1);
            assert_eq!(rtx_header.header_type, SessionHeaderType::RtxRequest.into());
        }

        time::sleep(Duration::from_millis(1000)).await;

        let expected_msg = "pacekt 1 lost, not retry left";
        assert!(logs_contain(expected_msg));
        let expected_msg = "a message was definitely lost in session 0";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_rtx_reception() {
        let (tx_gw, mut rx_gw) = tokio::sync::mpsc::channel(1);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(1);

        let session_config = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "local_agent", 0),
        };

        let session = Streaming::new(120, session_config, SessionDirection::Sender, tx_gw, tx_app);

        let mut message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
        header.session_id = 120;

        let session_msg: SessionMessage = SessionMessage::new(message.clone(), Info::new(120));

        // send 3 messages
        for _ in 0..3 {
            let res = session
                .on_message(session_msg.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the gw channel
        for i in 0..3 {
            let msg = rx_gw.recv().await.unwrap().unwrap();
            let msg_header = utils::get_session_header(&msg).unwrap();
            assert_eq!(msg_header.session_id, 120);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(msg_header.header_type, SessionHeaderType::Stream.into());
        }

        // receive an RTX for message 2
        let rtx = create_rtx_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            true,
            1,
            2,
            Some(vec![]),
        );

        let session_msg: SessionMessage = SessionMessage::new(rtx.clone(), Info::new(120));

        // send the RTX from the gw
        let res = session
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // get rtx reply message from gw
        let msg = rx_gw.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg).unwrap();
        assert_eq!(msg_header.session_id, 120);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(msg_header.header_type, SessionHeaderType::RtxReply.into());
        assert_eq!(
            utils::get_payload_from_msg(&msg).unwrap(),
            vec![0x1, 0x2, 0x3, 0x4]
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_e2e_with_losses() {
        let (tx_gw_sender, mut rx_gw_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_gw_receiver, mut rx_gw_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let session_config_sender = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "local_agent", 0),
        };

        let session_config_receiver = StreamingConfiguration {
            max_retries: 5,
            timeout: std::time::Duration::from_millis(500),
            source: encoder::encode_agent("cisco", "default", "remote_agent", 0),
        };

        let sender = Streaming::new(
            0,
            session_config_sender,
            SessionDirection::Sender,
            tx_gw_sender,
            tx_app_sender,
        );
        let receiver = Streaming::new(
            0,
            session_config_receiver,
            SessionDirection::Receiver,
            tx_gw_receiver,
            tx_app_receiver,
        );

        let message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        let session_msg: SessionMessage = SessionMessage::new(message.clone(), Info::new(0));
        // send 3 messages from the producer app
        // send 3 messages
        for _ in 0..3 {
            let res = sender
                .on_message(session_msg.clone(), MessageDirection::South)
                .await;
            assert!(res.is_ok());
        }

        // read the 3 messages from the sender gw channel
        // forward message 1 and 3 to the receiver
        for i in 0..3 {
            let msg = rx_gw_sender.recv().await.unwrap().unwrap();
            let msg_header = utils::get_session_header(&msg).unwrap();
            assert_eq!(msg_header.session_id, 0);
            assert_eq!(msg_header.message_id, i);
            assert_eq!(msg_header.header_type, SessionHeaderType::Stream.into());

            // the receiver should detect a loss for packet 1
            if i != 1 {
                let session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
                let res = receiver
                    .on_message(session_msg.clone(), MessageDirection::North)
                    .await;
                assert!(res.is_ok());
            }
        }

        // the receiver app should get the packet 0
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg.message).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 0);
        assert_eq!(msg_header.header_type, SessionHeaderType::Stream.into());

        // get the RTX and drop the first one before send it to sender
        let msg = rx_gw_receiver.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(msg_header.header_type, SessionHeaderType::RtxRequest.into());

        let msg = rx_gw_receiver.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(msg_header.header_type, SessionHeaderType::RtxRequest.into());

        // send the second reply to the producer
        let session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
        let res = sender
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // this should generate an RTX reply
        let msg = rx_gw_sender.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(msg_header.header_type, SessionHeaderType::RtxReply.into());

        let session_msg: SessionMessage = SessionMessage::new(msg.clone(), Info::new(0));
        let res = receiver
            .on_message(session_msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        // the receiver app should get the packet 1 and 2, packet 1 is an RTX
        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg.message).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 1);
        assert_eq!(msg_header.header_type, SessionHeaderType::RtxReply.into());

        let msg = rx_app_receiver.recv().await.unwrap().unwrap();
        let msg_header = utils::get_session_header(&msg.message).unwrap();
        assert_eq!(msg_header.session_id, 0);
        assert_eq!(msg_header.message_id, 2);
        assert_eq!(msg_header.header_type, SessionHeaderType::Stream.into());
    }
}
