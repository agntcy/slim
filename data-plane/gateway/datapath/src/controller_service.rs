// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//use std::net::SocketAddr;
use std::pin::Pin;

use crate::commands::ControlCommand;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::controller::proto::controller::v1::controller_service_server::ControllerService as GrpcControllerService;
use crate::controller::proto::controller::v1::Command;

#[derive(Debug, Clone)]
pub struct ControllerService {
    control_tx: mpsc::Sender<ControlCommand>,
}

impl ControllerService {
    pub fn new(control_tx: mpsc::Sender<ControlCommand>) -> Self {
        ControllerService { control_tx }
    }

}

#[tonic::async_trait]
impl GrpcControllerService for ControllerService {
    type OpenChannelStream = Pin<Box<dyn Stream<Item = Result<Command, Status>> + Send + 'static>>;

    async fn open_channel(
        &self,
        request: Request<tonic::Streaming<Command>>,
    ) -> Result<Response<Self::OpenChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        //self.process_stream(stream, CancellationToken::new());

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}
