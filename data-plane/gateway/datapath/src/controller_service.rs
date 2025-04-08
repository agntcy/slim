// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

//use agp_config::grpc::client::ClientConfig;

use crate::errors::DataPathError;
//use tonic::codegen::{Body, StdError};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::controller::proto::controller::v1::controller_service_server::ControllerService as GrpcControllerService;
use crate::controller::proto::controller::v1::Message;
use crate::pubsub::proto::pubsub::v1::Message as PubsubMessage;

#[derive(Debug, Clone)]
pub struct ControllerService {
    internal: Arc<ControllerServiceInternal>,
}

#[derive(Debug, Clone)]
struct ControllerServiceInternal {
    mp_tx: mpsc::Sender<Result<PubsubMessage, Status>>,
    mp_rx: Arc<mpsc::Receiver<Result<PubsubMessage, Status>>>,
}

impl ControllerService {
    pub fn new(
        mp_tx: mpsc::Sender<Result<PubsubMessage, Status>>,
        mp_rx: mpsc::Receiver<Result<PubsubMessage, Status>>,
    ) -> Self {
        let internal = ControllerServiceInternal {
            mp_tx: mp_tx,
            mp_rx: Arc::new(mp_rx),
        };

        ControllerService {
            internal: Arc::new(internal),
        }
    }

    async fn handle_new_message(
        &self,
        mut msg: Message,
    ) -> Result<(), DataPathError> {
        Ok(())
    }

    /*pub async fn connect<C>(
        &self,
        channel: C,
    ) -> Result<(tokio::task::JoinHandle<()>, u64), DataPathError>
    where
        C: tonic::client::GrpcService<tonic::body::BoxBody>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        //TODO
    }*/

    fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
        cancellation_token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            let mut try_to_reconnect = true;
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
                                        if let Err(e) = self_clone.handle_new_message(msg).await {
                                            error!("error processing incoming message");
                                        }
                                    }
                                    Err(e) => {
                                        error!("error receiving messages {:?}", e);
                                        break;
                                    }
                                }
                            }
                            None => {
                                debug!("end of stream");
                                break;
                            }
                        }
                    }
                    _ = token_clone.cancelled() => {
                        info!("shutting down stream on cancellation token");
                        break;
                    }
                }
            }
        });

        handle
    }
}

#[tonic::async_trait]
impl GrpcControllerService for ControllerService {
    type OpenChannelStream = Pin<Box<dyn Stream<Item = Result<Message, Status>> + Send + 'static>>;

    async fn open_channel(
        &self,
        request: Request<tonic::Streaming<Message>>,
    ) -> Result<Response<Self::OpenChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        self.process_stream(stream, CancellationToken::new());

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}
