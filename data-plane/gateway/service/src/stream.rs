// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

use crate::{
    producer_buffer, receiver_buffer,
    session::{Common, Error, Id, Info, Session, SessionDirection, SessionType, State},
    timer::{Timer, TimerObserver},
    MessageDirection,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;
use agp_datapath::{
    messages::{
        utils::{
            create_rtx_publication, get_message_as_publish, get_payload, get_rtx_id,
            get_session_header_type, get_session_id, get_source, service_type_to_int, set_msg_id,
            set_session_type,
        },
        Agent, AgentType,
    },
    pubsub::proto::pubsub::v1::Message,
};

use tonic::{async_trait, Status};
use tracing::{debug, error, info, trace, warn};

#[allow(dead_code)]
struct RtxTimerObserver {
    channel: mpsc::Sender<Result<(u32, bool), Status>>,
}

#[async_trait]
impl TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        debug!("timeout number {} for rtx {}, retry", timeouts, timer_id);

        // notify the process loop
        if self.channel.send(Ok((timer_id, true))).await.is_err() {
            error!("error notifing the process loop");
        }
    }

    async fn on_failure(&self, timer_id: u32, timeouts: u32) {
        debug!(
            "timeout number {} for rtx {}, stop retry",
            timeouts, timer_id
        );

        // notify the process loop
        if self.channel.send(Ok((timer_id, false))).await.is_err() {
            error!("error notifing the process loop");
        }
    }

    async fn on_stop(&self, timer_id: u32) {
        debug!("timer for rtx {} cancelled", timer_id);
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
pub(crate) struct Stream {
    common: Common,
    buffer: Arc<RwLock<Endpoint>>,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

#[allow(dead_code)]
impl Stream {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
    ) -> Stream {
        let (tx, rx) = mpsc::channel(128);
        let (timer_tx, timer_rx) = mpsc::channel(128);
        if session_direction == SessionDirection::Sender {
            let prod = Producer {
                buffer: ProducerBuffer::with_capacity(500),
                next_id: 0,
            };
            let stream = Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
                buffer: Arc::new(RwLock::new(Endpoint::Producer(prod))),
                tx,
            };
            stream.process_message(rx, timer_rx);
            stream
        } else {
            let observer = RtxTimerObserver { channel: timer_tx };

            let recv = Receiver {
                buffer: ReceiverBuffer::default(),
                timer_observer: Arc::new(observer),
                rtx_map: HashMap::new(),
                timers_map: HashMap::new(),
            };

            let stream = Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
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
        let state = self.common.state().clone();
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
                                                let pub_msg_opt = get_message_as_publish(&msg);
                                                if pub_msg_opt.is_none() {
                                                    error!("received invalid packet type on session {}: the packet is not a publication", session_id);
                                                    continue;
                                                }
                                                let pub_msg = pub_msg_opt.unwrap();
                                                match get_session_header_type(pub_msg){
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

                                                match get_session_id(pub_msg) {
                                                    Ok(id) => id,
                                                    Err(_) => {
                                                        error!("received packet for invalid session on session {}", session_id);
                                                        continue;
                                                    }
                                                };

                                                let msg_rtx = match get_rtx_id(pub_msg) {
                                                    Ok(rtx_opt) => match rtx_opt {
                                                        Some(rtx) => rtx,
                                                        None => {
                                                            error!("error parsing request RTX message on session {}", session_id);
                                                            continue;
                                                        },
                                                    },
                                                    Err(_) => {
                                                        error!("error parsing request RTX message on session {}", session_id);
                                                        continue;
                                                    }
                                                };

                                                // search the packet in the producer buffer
                                                let rtx_pub = match producer.buffer.get(msg_rtx as usize) {
                                                    Some(packet) => {
                                                        // the packet exists, send it to the source of the RTX
                                                        let (src_type, src_id) = match get_source(&msg) {
                                                            Ok((src_type, src_id)) => (src_type, src_id),
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
                                                        let rtx = create_rtx_publication(&Agent::default(), &src_type, src_id, false, session_id, msg_rtx, Some(get_payload(&packet).to_vec()));
                                                        rtx
                                                    }
                                                    None => {
                                                        // the packet does not exist so do nothing
                                                        // TODO(micpapal): improve by returning a rtx nack so that the remote app does not
                                                        // wait too long for all the retransmissions
                                                        debug!("received and RTX messages for an old packet on session {}", session_id);
                                                        continue;
                                                    },
                                                };

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
                                                let pub_msg = get_message_as_publish(&msg);
                                                if !producer.buffer.push(pub_msg.unwrap().clone()) {
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
                                        // here the packets are coming only from the gateway
                                        trace!("received message from the gataway on session {}, add it to the receiver buffer", session_id);
                                        match receiver.buffer.on_received_message(msg){
                                            Ok((recv, rtx)) =>{
                                                for opt in recv {
                                                    trace!("send recv packet to the application on session {}", session_id);
                                                    let info = Info::new(
                                                        session_id,
                                                        SessionType::Streaming,
                                                        state.clone(),
                                                    );

                                                    match opt {
                                                        Some(m) => {
                                                            // send message to the app
                                                            if send_app.send((m, info)).await.is_err() {
                                                                error!("error sending packet to the gateway on session {}", session_id);
                                                                // TODO: do we need to notify the app with an error packet?
                                                            }
                                                        }
                                                        None => {
                                                            // TODO. how do we notify the app that a message is lost forever
                                                            warn!("a message was definitely lost in session {}", session_id);
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
                                                        error!("error sending RTX for id {} on session {}", r, session_id);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error!("error adding message to the buffer: {}", e.to_string());
                                                continue; // TODO: what happen here? the packet will be never sent to app
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
                                            // send the RTX again
                                            let rtx = match receiver.rtx_map.get(&msg_id) {
                                                Some(rtx) => rtx,
                                                None => {
                                                    error!("rtx message does not exist in the map, skip retransmission and try to stop the timer");
                                                    let timer = match receiver.timers_map.get(&msg_id) {
                                                        Some(t) => t,
                                                        None => {
                                                            error!("timer notfound");
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
                                            receiver.rtx_map.remove(&msg_id);
                                            receiver.timers_map.remove(&msg_id);

                                            match receiver.buffer.on_lost_message(msg_id) {
                                                Ok(recv) => {
                                                    for opt in recv {
                                                        let info = Info::new(
                                                            session_id,
                                                            SessionType::Streaming,
                                                            state.clone(),
                                                        );

                                                        match opt {
                                                            Some(m) => {
                                                                // send message to the app
                                                                if send_app.send((m, info)).await.is_err() {
                                                                    error!("error sending packet to the gateway on session {}", session_id);
                                                                    // TODO: do we need to notify the app with an error packet?
                                                                }
                                                            }
                                                            None => {
                                                                // TODO. how do we notify the app that a message is lost forever
                                                                warn!("a message was definitely lost in session {}", session_id);
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
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

impl Session for Stream {
    fn id(&self) -> Id {
        self.common.id()
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_type(&self) -> SessionType {
        SessionType::Streaming
    }

    fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        debug!("receive message");
        let tx = self.tx.clone();
        Box::pin(async move {
            tx.send(Ok((message, direction)))
                .await
                .map_err(|e| Error::GatewayTransmission(e.to_string())) // TODO put the right error here
        })
    }
}

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

        let session = Stream::new(0, SessionDirection::Sender, tx_gw.clone(), tx_app.clone());

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(session.session_type(), SessionType::Streaming);

        let session = Stream::new(1, SessionDirection::Receiver, tx_gw, tx_app);

        assert_eq!(session.id(), 1);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(session.session_type(), SessionType::Streaming);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_sender_and_receiver_on_message() {
        let (tx_gw_sender, mut rx_gw_sender) = tokio::sync::mpsc::channel(1);
        let (tx_app_sender, _rx_app_sender) = tokio::sync::mpsc::channel(1);

        let (tx_gw_receiver, _rx_gw_receiver) = tokio::sync::mpsc::channel(1);
        let (tx_app_receiver, mut rx_app_receiver) = tokio::sync::mpsc::channel(1);

        let sender = Stream::new(0, SessionDirection::Sender, tx_gw_sender, tx_app_sender);
        let receiver = Stream::new(
            0,
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
        header.session_id = 1;

        // set session header type for test check
        let mut expected_msg = message.clone();
        let _ = set_session_type(&mut expected_msg, SessionHeaderType::Stream);

        // send a message from the sender app to the gw
        let res = sender
            .on_message(message.clone(), MessageDirection::South) // !!!! WHAT IS THE RIGHT DIRECTION
            .await;
        assert!(res.is_ok());

        let msg = rx_gw_sender.recv().await.unwrap().unwrap();
        assert_eq!(msg, expected_msg);

        // send the same message to the receiver
        let res = receiver
            .on_message(msg.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        let (msg, info) = rx_app_receiver.recv().await.unwrap();
        assert_eq!(msg, expected_msg);
        assert_eq!(info.id, 0);
        assert_eq!(info.session_type, SessionType::Streaming);
        assert_eq!(info.state, State::Active);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stream_rtx_timeouts() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session = Stream::new(0, SessionDirection::Receiver, tx_gw.clone(), tx_app.clone());

        assert_eq!(session.id(), 0);
        assert_eq!(session.state(), &State::Active);
        assert_eq!(session.session_type(), SessionType::Streaming);

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
        header.session_id = 1;

        let res = session
            .on_message(message.clone(), MessageDirection::North)
            .await;
        assert!(res.is_ok());

        time::sleep(Duration::from_millis(500)).await;
        
    }

    // test RTX with producer buffer
}
