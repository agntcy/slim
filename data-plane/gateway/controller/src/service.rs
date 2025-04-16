// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::pin::Pin;
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

#[derive(Debug, Clone)]
pub struct ControllerService {
}

impl ControllerService {
    pub fn new() -> Self {
        ControllerService { }
    }

    async fn handle_new_message(
        &self,
        msg: ControlMessage,
    ) -> Result<(), ControllerError> {
        match msg.payload {
            Some(ref payload) => {
                match payload {
                    crate::api::proto::api::v1::control_message::Payload::ConfigCommand(config) => {
                        println!("Processing configuration command: {:?}", config);
                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                        };

                        /*let reply = ControlMessage { message_id: (), payload: () } {
                            message_id: generate_message_id(),
                            payload: Some(
                                crate::api::proto::api::v1::command::Payload::Ack(ack)
                            ),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            eprintln!("Failed to send ACK: {}", e);
                        }*/
                    },
                    crate::api::proto::api::v1::control_message::Payload::Ack(ack) => {
                        // received an ack, do nothing
                    },
                }
            },
            None => {
                println!("Server received command {} with no payload", msg.message_id);
            }
        }
                
        Ok(())
    }

    async fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
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
                                        if let Err(e) = svc.handle_new_message(msg).await {
                                            //TODO: handle error
                                        }
                                    }
                                    Err(e) => {
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

        let (tx, rx) = mpsc::channel(128);
        match client
                .open_control_channel(Request::new(ReceiverStream::new(rx)))
                .await
            {
                Ok(stream) => {
                    let ret = self.process_stream(stream.into_inner()).await;

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
        let (_, rx) = mpsc::channel(128);

        self.process_stream(stream).await;

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
