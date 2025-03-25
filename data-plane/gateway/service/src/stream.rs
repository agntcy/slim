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

use agp_datapath::{messages::utils::{set_msg_id, set_session_type}, pubsub::proto::pubsub::v1::Message};
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;
use agp_datapath::{
    messages::utils::{self, get_message_as_publish},
    pubsub::proto::pubsub::v1::message,
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
    tx: mpsc::Sender<Result<Message, Status>>,
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

    fn process_message (&self, direction: MessageDirection, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let buffer_clone =  self.buffer.clone();
        let session_id = self.common.id();
        let send_gw = self.common.tx_gw();
        let send_app = self.common.tx_app();
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
                            Some(msg) => {
                                match msg {
                                    Ok(mut msg) => {
                                        debug!("received message in stream session {}, direction {:?}", session_id, direction);
                                        match &mut *buffer_clone.write().await {
                                            Endpoint::Receiver(recv) => {
                                                if direction == MessageDirection::South {
                                                    // this should never happen
                                                    // TODO break with error
                                                    error!("received message from the wrong direction, expected north got south");
                                                }
                                                match recv.buffer.on_received_message(msg){
                                                    Ok((recv, rtx)) =>{
                                                        for m in recv {
                                                            // TODO send to the app
                                                        }
                                                        for r in rtx {
                                                            // TODO send RTX
                                                        }
                                                    }
                                                    Err(_) => {
                                                        // TODO break with error
                                                        error!("error adding message to the buffer");
                                                    }
                                                }
                                                
                                            }
                                            Endpoint::Procucer(prod) => {
                                                if direction == MessageDirection::North {
                                                    // this should never happen
                                                    // TODO break with error
                                                    error!("received message from the wrong direction, expected south got north");
                                                }

                                                if set_session_type(&mut msg, SessionHeaderType::Stream).is_err() {
                                                    // TODO break with error
                                                    error!("error setting session type");
                                                }

                                                if set_msg_id(&mut msg, prod.next_id).is_err() {
                                                    // TODO break with error
                                                    error!("error setting session type");
                                                }
                                               
                                                prod.next_id += 1;

                                                // TODO put this inside prod_buffer
                                                let pub_msg = get_message_as_publish(&msg); 

                                                if !prod.buffer.push(pub_msg.unwrap().clone()) {
                                                    warn!("cannot add packet to the local buffer");
                                                }

                                                send_gw.send(Ok(msg)).await; // TODO check for error
                                            }
                                            _ => {
                                                // TODO break with error
                                                error!("cannot add packet to the local buffer");
                                            }
                                        }
                        
                                    }
                                    Err(e) => {
                                        error!("error receiving message: {}", e);
                                    }
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
        mut message: Message,
        direction: MessageDirection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        return Box::pin(async move {
            self.tx.send(Ok(message))
                .await
                .map_err(|e| Error::GatewayTransmission(e.to_string()))
        });
    }
}