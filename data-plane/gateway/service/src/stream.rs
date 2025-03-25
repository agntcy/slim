// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{
    producer_buffer, receiver_buffer,
    session::{Common, Error, Id, Info, Session, SessionDirection, SessionType, State},
    MessageDirection,
};
use producer_buffer::ProducerBuffer;
use receiver_buffer::ReceiverBuffer;

use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;
use agp_datapath::{
    messages::utils::{self, get_message_as_publish},
    pubsub::proto::pubsub::v1::message,
};
use tonic::Status;
use tracing::warn;

struct Producer {
    buffer: ProducerBuffer,
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
    buffer: Endpoint,
    next_id: u32, // id of the next packet to send
}

impl Stream {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
    ) -> Stream {
        if session_direction == SessionDirection::Sender {
            let prod = Producer {
                buffer: ProducerBuffer::with_capacity(500),
            };

            Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
                buffer: Endpoint::Procucer(prod),
                next_id: 0,
            }
        } else {
            let recv = Receiver {
                buffer: ReceiverBuffer::default(),
            };

            Stream {
                common: Common::new(id, session_direction, tx_gw, tx_app),
                buffer: Endpoint::Receiver(recv),
                next_id: 0,
            }
        }
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
        &mut self,
        mut message: Message,
        direction: MessageDirection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        // set the session type
        let header = utils::get_session_header_as_mut(&mut message);
        if header.is_none() {
            return Box::pin(
                async move { Err(Error::AppTransmission("missing header".to_string())) },
            );
        }

        header.unwrap().header_type = utils::service_type_to_int(SessionHeaderType::Stream);

        // clone tx
        match direction {
            MessageDirection::North => {
                // TODO: add to the receiver buffer
                match &mut self.buffer {
                    Endpoint::Receiver(recv) => {
                        match recv.buffer.on_received_message(message) {
                            Ok(vec) => {
                                let tx = self.common.tx_app();
                                for msg in vec {
                                    if msg.is_some() {
                                        // create info
                                        let info = Info::new(
                                            self.common.id(),
                                            SessionType::Streaming,
                                            self.common.state().clone(),
                                        );

                                       
                                        //Box::pin(async move {
                                        let res = tx.send((message, info))
                                                .await;
                                                //.map_err(|e| Error::AppTransmission(e.to_string()))
                                        //});
                                    }
                                }
                                Box::pin(async move { Ok(()) })
                            }
                            Err(_) => {
                                return Box::pin(async move {
                                    Err(Error::AppTransmission(
                                        "error while processing received messages".to_string(),
                                    ))
                                });
                            }
                        }
                    }
                    _ => {
                        return Box::pin(async move {
                            Err(Error::AppTransmission(
                                "error getting local receiver buffer".to_string(),
                            ))
                        });
                    }
                }
            }
            MessageDirection::South => {
                // add next id to the heade
                let header = utils::get_session_header_as_mut(&mut message).unwrap();
                header.message_id = self.next_id;
                self.next_id += 1;

                // store the packet in the local buffer
                match &self.buffer {
                    Endpoint::Procucer(prod) => {
                        let msg = get_message_as_publish(&message);

                        if !prod.buffer.push(msg.unwrap().clone()) {
                            warn!("cannot add packet to the local buffer");
                        }
                    }
                    _ => {
                        warn!("cannot add packet to the local buffer");
                    }
                }

                let tx = self.common.tx_gw();
                Box::pin(async move {
                    tx.send(Ok(message))
                        .await
                        .map_err(|e| Error::GatewayTransmission(e.to_string()))
                })
            }
        }
    }
}