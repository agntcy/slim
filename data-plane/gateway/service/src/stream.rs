// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::{
    producer_buffer, receiver_buffer,
    session::{Common, Error, Id, Info, Session, SessionDirection, SessionType, State},
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

use tonic::Status;
use tracing::{debug, error, info, warn};

struct Producer {
    buffer: ProducerBuffer,
    next_id: u32,
}

struct Receiver {
    buffer: ReceiverBuffer,
}

enum Endpoint {
    Procucer(Producer),
    Receiver(Receiver),
    Unknown,
}

pub(crate) struct Stream {
    common: Common,
    buffer: Arc<RwLock<Endpoint>>,
    tx: mpsc::Sender<Result<(Message, MessageDirection), Status>>,
}

impl Stream {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
    ) -> Stream {
        let (tx, rx) = mpsc::channel(128);
        if session_direction == SessionDirection::Sender {
            let prod = Producer {
                buffer: ProducerBuffer::with_capacity(500),
                next_id: 0,
            };
            let stream = Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
                buffer: Arc::new(RwLock::new(Endpoint::Procucer(prod))),
                tx,
            };
            stream.process_message(rx);
            stream
        } else {
            let recv = Receiver {
                buffer: ReceiverBuffer::default(),
            };

            let stream = Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
                buffer: Arc::new(RwLock::new(Endpoint::Receiver(recv))),
                tx,
            };

            stream.process_message(rx);
            stream
        }
    }

    fn process_message(&self, mut rx: mpsc::Receiver<Result<(Message, MessageDirection), Status>>) {
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
                                    Endpoint::Procucer(producer) => {
                                        match direction {
                                            MessageDirection::North => {
                                                // received a message from the GW
                                                // this must be an RTX message otherwise drop it
                                                let pub_msg_opt = get_message_as_publish(&msg);
                                                if pub_msg_opt.is_none() {
                                                    error!("received invailid packet type on session {}: the packet is not a publication", session_id);
                                                    continue;
                                                }
                                                let pub_msg = pub_msg_opt.unwrap();
                                                match get_session_header_type(pub_msg){
                                                    Ok(session_type) => {
                                                        if (session_type != service_type_to_int(SessionHeaderType::RtxRequest)) {
                                                            error!("received invailid packet type on session {}: not RTX request", session_id);
                                                            continue;
                                                        }
                                                        session_type
                                                    }
                                                    Err(_) => {
                                                        error!("received invailid packet type on session {}: missing header type", session_id);
                                                        continue;
                                                    }
                                                };

                                                match get_session_id(&pub_msg) {
                                                    Ok(id) => id,
                                                    Err(_) => {
                                                        error!("received for the wrong session on session {}", session_id);
                                                        continue;
                                                    }
                                                };

                                                let msg_rtx = match get_rtx_id(&pub_msg) {
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
                                                        // the packet exists, send it to the sorce of the RTX
                                                        let (src_type, src_id) = match get_source(&msg) {
                                                            Ok((src_type, src_id)) => (src_type, src_id),
                                                            Err(_) => {
                                                                error!("error parsing recived RTX on session {}: missing source", session_id);
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
                                                        debug!("recevied and RTX messages for an old packet on session {}", session_id);
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
                                                error!("error adding message to the buffer");
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
