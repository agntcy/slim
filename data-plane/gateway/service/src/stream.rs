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
use tracing::{debug, error, info, warn};

#[allow(dead_code)]
struct RtxTimerObserver {
    channel: mpsc::Sender<Result<(u32, bool), Status>>,
}

#[async_trait]
impl TimerObserver for RtxTimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        debug!("timeout number {} for rtx {}, retry", timeouts, timer_id);

        // notify the process loop
        match self.channel.send(Ok((timer_id, true))).await {
            Ok(_) => {}
            Err(e) => {
                error!("error notifing the process loop: {}", e.to_string());
                return;
            }
        }
    }

    async fn on_failure(&self, timer_id: u32, timeouts: u32) {
        debug!(
            "timeout number {} for rtx {}, stop retry",
            timeouts, timer_id
        );

        // notify the process loop
        match self.channel.send(Ok((timer_id, false))).await {
            Ok(_) => {}
            Err(e) => {
                error!("error notifing the process loop: {}", e.to_string());
                return;
            }
        }
    }

    async fn on_stop(&self, timer_id: u32) {
        debug!("timer for rtx {} cancelled", timer_id);
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
        tokio::spawn(async move {
            debug!("starting message processing in session id {}", session_id);
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                info!("no more messages to process");
                                break;
                            }
                            Some(result) => {
                                if result.is_err() {
                                    error!("error receiving a message, drop it");
                                    continue;
                                }
                                let (mut msg, direction) = result.unwrap();
                                match &mut *buffer_clone.write().await {
                                    Endpoint::Producer(producer) => {
                                        match direction {
                                            MessageDirection::North => {
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
                                                        error!("received for the wrong session on session {}", session_id);
                                                        continue;
                                                    }
                                                };

                                                let msg_rtx = match get_rtx_id(pub_msg) {
                                                    Ok(rtx_opt) => match rtx_opt {
                                                        Some(rtx) => rtx,
                                                        None => {
                                                            error!("error parsion RTX message on session {}", session_id);
                                                            continue;
                                                        },
                                                    },
                                                    Err(_) => {
                                                        error!("error parsion RTX message on session {}", session_id);
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

                                                match send_gw.send(Ok(rtx_pub)).await {
                                                    Ok(_) => {},
                                                    Err(e) => {
                                                        error!("error sending packet to the gateway: {}", e.to_string());
                                                        continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
                                                    }
                                                }
                                            }
                                            MessageDirection::South => {
                                                // received a message from the APP
                                                // set the session header, add the message to the buffer and send it
                                                if set_session_type(&mut msg, SessionHeaderType::Stream).is_err() {
                                                    error!("error setting session type, drop packet");
                                                    continue; // TO BE DISCUSSED, is it ok to drop the packet? do we need to notify the app with an error packet?
                                                }

                                                if set_msg_id(&mut msg, producer.next_id).is_err() {
                                                    error!("error setting msg id. drop packet");
                                                    continue; // TO BE DISCUSSED, is it ok to drop the packet? do we need to notify the app with an error packet?
                                                }

                                                producer.next_id += 1;

                                                let pub_msg = get_message_as_publish(&msg);
                                                if !producer.buffer.push(pub_msg.unwrap().clone()) {
                                                    warn!("cannot add packet to the local buffer");
                                                }

                                                match send_gw.send(Ok(msg)).await {
                                                    Ok(_) => {},
                                                    Err(e) => {
                                                        error!("error sending packet to the gateway: {}", e.to_string());
                                                        continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Endpoint::Receiver(receiver) => {
                                        // here the packets are coming only from the gateway
                                        match receiver.buffer.on_received_message(msg){
                                            Ok((recv, rtx)) =>{
                                                for m in recv {
                                                    let info = Info::new(
                                                        session_id,
                                                        SessionType::Streaming,
                                                        state.clone(),
                                                    );

                                                    // here a message can be NONE
                                                    match send_app.send((m.unwrap(), info)).await {
                                                        Ok(_) => {},
                                                        Err(e) => {
                                                            error!("error sending message to the app {}", e.to_string());
                                                            continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
                                                        }
                                                    }
                                                }
                                                for r in rtx {
                                                    debug!("send a rtx for message {}", r);
                                                    // TODO get the agent name
                                                    // TODO get the destination
                                                    let rtx = create_rtx_publication(&Agent::default(), &AgentType::default(), None, true, session_id, r, Some(vec![]));

                                                    // set state for RTX
                                                    let timer = Timer::new(r, 500, 5);
                                                    timer.start(receiver.timer_observer.clone());

                                                    receiver.rtx_map.insert(r, rtx.clone());
                                                    receiver.timers_map.insert(r, timer);

                                                    match send_gw.send(Ok(rtx)).await {
                                                        Ok(_) => {},
                                                        Err(e) => {
                                                            error!("error sending packet to the gateway: {}", e.to_string());
                                                            continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
                                                        }
                                                    }

                                                }
                                            }
                                            Err(e) => {
                                                error!("error adding message to the buffer: {}", e.to_string());
                                                continue; // TO BE DISCUSSED, what to do here?
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
                    next_timer = timer_rx.recv() => {
                        match next_timer {
                            None => {
                                info!("no more timers to process");
                                break; // should we break here?
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
                                            match send_gw.send(Ok(rtx.clone())).await {
                                                Ok(_) => {},
                                                Err(e) => {
                                                    error!("error sending rtx the gateway: {}", e.to_string());
                                                    continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
                                                }
                                            }
                                        } else {
                                            receiver.rtx_map.remove(&msg_id);
                                            receiver.timers_map.remove(&msg_id);

                                            match receiver.buffer.on_lost_message(msg_id) {
                                                Ok(recv) => {
                                                    for m in recv {
                                                        let info = Info::new(
                                                            session_id,
                                                            SessionType::Streaming,
                                                            state.clone(),
                                                        );

                                                        // here a message can be None
                                                        match send_app.send((m.unwrap(), info)).await {
                                                            Ok(_) => {},
                                                            Err(e) => {
                                                                error!("error sending message to the app {}", e.to_string());
                                                                continue; // TO BE DISCUSSED, what to do here? do we need to notify the app with an error packet?
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
                                //if retry {

                                //}
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
        let tx = self.tx.clone();
        Box::pin(async move {
            tx.send(Ok((message, direction)))
                .await
                .map_err(|e| Error::GatewayTransmission(e.to_string())) // TODO put the right error here
        })
    }
}
