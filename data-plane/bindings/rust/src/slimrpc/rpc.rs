// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::context::{MessageContext, SessionContext};
use super::error::Result;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

pub type RequestStream<T> = Pin<Box<dyn Stream<Item = Result<(T, MessageContext)>> + Send>>;
pub type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RPCHandlerType {
    UnaryUnary,
    UnaryStream,
    StreamUnary,
    StreamStream,
}

#[async_trait]
pub trait UnaryUnaryHandler<Req, Res>: Send + Sync {
    async fn call(
        &self,
        request: Req,
        msg_ctx: MessageContext,
        session_ctx: SessionContext,
    ) -> Result<Res>;
}

#[async_trait]
pub trait UnaryStreamHandler<Req, Res>: Send + Sync {
    async fn call(
        &self,
        request: Req,
        msg_ctx: MessageContext,
        session_ctx: SessionContext,
    ) -> Result<ResponseStream<Res>>;
}

#[async_trait]
pub trait StreamUnaryHandler<Req, Res>: Send + Sync {
    async fn call(
        &self,
        request_stream: RequestStream<Req>,
        session_ctx: SessionContext,
    ) -> Result<Res>;
}

#[async_trait]
pub trait StreamStreamHandler<Req, Res>: Send + Sync {
    async fn call(
        &self,
        request_stream: RequestStream<Req>,
        session_ctx: SessionContext,
    ) -> Result<ResponseStream<Res>>;
}

pub enum RPCHandler {
    UnaryUnary(Box<dyn UnaryUnaryHandler<Vec<u8>, Vec<u8>>>),
    UnaryStream(Box<dyn UnaryStreamHandler<Vec<u8>, Vec<u8>>>),
    StreamUnary(Box<dyn StreamUnaryHandler<Vec<u8>, Vec<u8>>>),
    StreamStream(Box<dyn StreamStreamHandler<Vec<u8>, Vec<u8>>>),
}

impl RPCHandler {
    pub fn handler_type(&self) -> RPCHandlerType {
        match self {
            RPCHandler::UnaryUnary(_) => RPCHandlerType::UnaryUnary,
            RPCHandler::UnaryStream(_) => RPCHandlerType::UnaryStream,
            RPCHandler::StreamUnary(_) => RPCHandlerType::StreamUnary,
            RPCHandler::StreamStream(_) => RPCHandlerType::StreamStream,
        }
    }
}
