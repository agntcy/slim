// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::pin::Pin;

use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};

use crate::controller::proto::controller::v1::controller_service_server::ControllerService;
use crate::controller::proto::controller::v1::Message;
use crate::pubsub::proto::pubsub::v1::Message as PubsubMessage;

pub struct Service {
    tx_gw: Mutex<Option<mpsc::Sender<Result<PubsubMessage, Status>>>>,
    rx_gw: Mutex<Option<mpsc::Receiver<Result<PubsubMessage, Status>>>>,
}

impl Service {
    pub fn new(
        tx_gw: mpsc::Sender<Result<PubsubMessage, Status>>,
        rx_gw: mpsc::Receiver<Result<PubsubMessage, Status>>,
    ) -> Self {
        Service {
            tx_gw: Mutex::new(Some(tx_gw)),
            rx_gw: Mutex::new(Some(rx_gw)),
        }
    }
}

#[tonic::async_trait]
impl ControllerService for Service {
    type OpenChannelStream = Pin<Box<dyn Stream<Item = Result<Message, Status>> + Send + 'static>>;

    async fn open_channel(
        &self,
        request: Request<tonic::Streaming<Message>>,
    ) -> Result<Response<Self::OpenChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        //self.process_stream(stream, None, CancellationToken::new(), false);

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}
