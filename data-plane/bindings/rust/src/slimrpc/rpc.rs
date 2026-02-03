// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::context::{MessageContext, SessionContext};
use super::error::Result;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

pub type RequestStream<T> = Pin<Box<dyn Stream<Item = Result<(T, MessageContext)>> + Send>>;
pub type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RpcHandlerType {
    UnaryUnary,
    UnaryStream,
    StreamUnary,
    StreamStream,
}

// UniFFI callback interface for foreign language implementations
// Note: This will be defined in a .udl file or via proc-macro in a future update
// For now, we keep the Rust-only handler traits

// Internal Rust async traits (not exported via UniFFI)
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

pub enum RpcHandler {
    UnaryUnary(Box<dyn UnaryUnaryHandler<Vec<u8>, Vec<u8>>>),
    UnaryStream(Box<dyn UnaryStreamHandler<Vec<u8>, Vec<u8>>>),
    StreamUnary(Box<dyn StreamUnaryHandler<Vec<u8>, Vec<u8>>>),
    StreamStream(Box<dyn StreamStreamHandler<Vec<u8>, Vec<u8>>>),
}

impl RpcHandler {
    pub fn handler_type(&self) -> RpcHandlerType {
        match self {
            RpcHandler::UnaryUnary(_) => RpcHandlerType::UnaryUnary,
            RpcHandler::UnaryStream(_) => RpcHandlerType::UnaryStream,
            RpcHandler::StreamUnary(_) => RpcHandlerType::StreamUnary,
            RpcHandler::StreamStream(_) => RpcHandlerType::StreamStream,
        }
    }
}
