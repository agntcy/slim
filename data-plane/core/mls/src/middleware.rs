// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tower::{Layer, Service};

use slim_datapath::api::proto::pubsub::v1::Message;
use slim_datapath::messages::Agent;

pub type OutboundRequest = (Agent, Message, Option<slim_service::session::Info>);
pub type OutboundResponse = ();

#[derive(Clone)]
pub struct MlsLayer {
    pub local_agent: Agent,
    pub group_id: Option<String>,
}

impl<S> Layer<S> for MlsLayer {
    type Service = MlsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MlsMiddleware {
            inner,
            local_agent: self.local_agent.clone(),
            group_id: self.group_id.clone(),
        }
    }
}

#[derive(Clone)]
pub struct MlsMiddleware<S> {
    inner: S,
    local_agent: Agent,
    group_id: Option<String>,
}

impl<S> Service<OutboundRequest> for MlsMiddleware<S>
where
    S: Service<OutboundRequest, Response = OutboundResponse, Error = Status>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = OutboundResponse;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<OutboundResponse, Status>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: OutboundRequest) -> Self::Future {
        let (ref source_agent, ref msg, _) = req;
        println!(
            "[Outbound Middleware] local_agent = {:?}, group_id = {:?}, publishing on behalf of {:?}",
            self.local_agent, self.group_id, source_agent
        );

        let mut inner_clone = self.inner.clone();
        let req_clone = (source_agent.clone(), msg.clone(), req.2.clone());
        Box::pin(async move { inner_clone.call(req_clone).await })
    }
}

pub type MlsClient = mls_rs::Client<mls_rs_crypto_awslc::AwsLcCryptoProvider>;

pub type InboundStream = ReceiverStream<Result<Message, Status>>;

#[derive(Clone)]
pub struct MlsInboundMiddleware<S> {
    inner: S,
    local_agent: Agent,
    group_id: Option<String>,
}

impl<S> MlsInboundMiddleware<S> {
    pub fn new(inner: S, local_agent: Agent, group_id: Option<String>) -> Self {
        MlsInboundMiddleware {
            inner,
            local_agent,
            group_id,
        }
    }
}

impl<S> Service<Request<tonic::Streaming<Message>>> for MlsInboundMiddleware<S>
where
    S: Service<Request<InboundStream>, Response = Response<InboundStream>, Error = Status>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<InboundStream>;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Status>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Status>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<tonic::Streaming<Message>>) -> Self::Future {
        let mut inbound_stream = req.into_inner();
        let local_agent = self.local_agent.clone();
        let group_id = self.group_id.clone();
        let mut inner_clone = self.inner.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            while let Some(item) = inbound_stream.message().await.transpose() {
                match item {
                    Ok(msg) => {
                        let sending_agent = msg.get_source().clone();
                        println!(
                            "[Inbound Middleware] local_agent = {:?}, group_id = {:?}, message from {:?}",
                            local_agent, group_id, sending_agent
                        );

                        if tx.send(Ok(msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
                }
            }
        });

        let outbound_stream = ReceiverStream::new(rx);
        let forward_req = Request::new(outbound_stream);
        Box::pin(async move { inner_clone.call(forward_req).await })
    }
}
