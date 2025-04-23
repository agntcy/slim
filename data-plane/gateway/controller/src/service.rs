// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Request, Response, Status};
use tonic::codegen::{Body, StdError};
use tracing::debug;

use crate::api::proto::api::v1::{
    Ack,
    ControlMessage,
    controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::errors::ControllerError;

use agp_datapath::message_processing::MessageProcessor;
use agp_datapath::pubsub::proto::pubsub::v1::Message as PubsubMessage;

#[derive(Debug, Clone)]
pub struct ControllerService {
    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// channel to send messages into the datapath
    tx_gw: mpsc::Sender<Result<PubsubMessage, Status>>
}

impl ControllerService {
    pub fn new(message_processor: Arc<MessageProcessor>) -> Self {
        let (_, tx_gw, _) = message_processor.register_local_connection();

        ControllerService {
            message_processor: message_processor,
            tx_gw,
        }
    }

    async fn handle_new_message(
        &self,
        msg: ControlMessage,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> Result<(), ControllerError> {
        match msg.payload {
            Some(ref payload) => {
                match payload {
                    crate::api::proto::api::v1::control_message::Payload::ConfigCommand(config) => {
                        println!("processing configuration command: {:?}", config);

                        for conn in &config.connections_to_create {
                            //print!(conn.)
                        }

                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            //TODO: add (error) message
                        };

                        let reply = ControlMessage {
                            message_id: generate_message_id(),
                            payload: Some(crate::api::proto::api::v1::control_message::Payload::Ack(ack))
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            eprintln!("failed to send ACK: {}", e);
                        }
                    },
                    crate::api::proto::api::v1::control_message::Payload::Ack(_ack) => {
                        // received an ack, do nothing
                    },
                }
            },
            None => {
                println!("received control message {} with no payload", msg.message_id);
            }
        }
                
        Ok(())
    }

    async fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> tokio::task::JoinHandle<()> {
        let svc = self.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
                                        if let Err(_e) = svc.handle_new_message(msg, tx.clone()).await {
                                            //TODO: handle error
                                        }
                                    }
                                    Err(_e) => {
                                        //TODO: handle error
                                    }
                                }
                            }
                            None => {
                                debug!("end of stream");
                                break;
                            }
                        }
                    }
                }
            }
        });

        handle
    }

    pub async fn connect<C>(
        &self,
        channel: C,
    ) -> Result<tokio::task::JoinHandle<()>, ControllerError>
    where
        C: tonic::client::GrpcService<tonic::body::Body>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        let mut client: ControllerServiceClient<C> = ControllerServiceClient::new(channel);

        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
        let out_stream = ReceiverStream::new(rx)
            .map(|res| res.expect("mapping error"));

        match client
                .open_control_channel(Request::new(out_stream))
                .await
            {
                Ok(stream) => {
                    let ret = self.process_stream(stream.into_inner(), tx).await;
                    return Ok(ret);
                }
                Err(_) => {
                    Err(ControllerError::ConnectionError(
                        "reached max connection retries".to_string(),
                    ))
                }
            }
    }
}

#[tonic::async_trait]
impl GrpcControllerService for ControllerService {
    type OpenControlChannelStream = Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send + 'static>>;

    async fn open_control_channel(
        &self,
        request: Request<tonic::Streaming<ControlMessage>>,
    ) -> Result<Response<Self::OpenControlChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);

        self.process_stream(stream, tx.clone()).await;

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenControlChannelStream
        ))
    }
}

fn generate_message_id() -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    format!("msg-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}
