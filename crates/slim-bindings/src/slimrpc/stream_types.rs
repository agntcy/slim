// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Stream wrapper types for SlimRPC UniFFI bindings
//!
//! The non-multicast stream types are re-exported directly from the
//! `agntcy-slim-rpc` crate. The multicast message types are kept here because
//! their FFI-facing versions expose `crate::Name` (a UniFFI `Record`) instead
//! of the native `slim_datapath` name used by `slim_rpc`. The multicast reader
//! and handler are thin newtype wrappers over their `slim_rpc` counterparts
//! that convert the native name into the FFI name on the way out.

use std::sync::Arc;

use slim_rpc::RpcError;

// Non-multicast UniFFI stream types come straight from `slim_rpc`.
pub use slim_rpc::{
    BidiStreamHandler, DecodedStream, RawStream, RequestStreamWriter, ResponseSink,
    ResponseStreamReader, StreamMessage, StreamSource, UniffiRequestStream as RequestStream,
};

// ── Multicast stream types ────────────────────────────────────────────────────

/// Per-message context for a multicast RPC response — identifies which group
/// member sent the response.
#[derive(uniffi::Record, Clone, Debug)]
pub struct RpcMessageContext {
    /// The SLIM name of the group member that sent this response.
    pub source: Arc<crate::Name>,
}

/// A single item in a multicast response stream, pairing the response payload
/// with the identity of the member that produced it.
#[derive(uniffi::Record, Clone, Debug)]
pub struct RpcMulticastItem {
    /// Context identifying the source member.
    pub context: RpcMessageContext,
    /// The encoded response payload (raw bytes).
    pub message: Vec<u8>,
}

/// Message from a multicast response stream.
#[derive(uniffi::Enum)]
pub enum MulticastStreamMessage {
    /// Successfully received response item with source context.
    Data { item: RpcMulticastItem },
    /// Error from one member — other members may still be active.
    Error { error: RpcError },
    /// All members have finished — the stream has ended.
    End,
}

/// Convert a `slim_rpc` multicast message (native name) into the FFI-facing
/// message (`crate::Name`).
fn to_ffi_message(msg: slim_rpc::MulticastStreamMessage) -> MulticastStreamMessage {
    match msg {
        slim_rpc::MulticastStreamMessage::Data { item } => MulticastStreamMessage::Data {
            item: RpcMulticastItem {
                context: RpcMessageContext {
                    source: Arc::new(crate::Name::from_slim_name(
                        item.context.source.as_ref().clone(),
                    )),
                },
                message: item.message,
            },
        },
        slim_rpc::MulticastStreamMessage::Error { error } => {
            MulticastStreamMessage::Error { error }
        }
        slim_rpc::MulticastStreamMessage::End => MulticastStreamMessage::End,
    }
}

/// Response stream reader for multicast RPC calls.
///
/// Thin wrapper over [`slim_rpc::MulticastResponseReader`] that converts each
/// item's native source name into the FFI [`crate::Name`].
#[derive(uniffi::Object)]
pub struct MulticastResponseReader(Arc<slim_rpc::MulticastResponseReader>);

impl MulticastResponseReader {
    /// Wrap a `slim_rpc` reader.
    pub(crate) fn new(inner: Arc<slim_rpc::MulticastResponseReader>) -> Self {
        Self(inner)
    }
}

#[uniffi::export]
impl MulticastResponseReader {
    /// Pull the next item from the multicast response stream (blocking).
    pub fn next(&self) -> MulticastStreamMessage {
        to_ffi_message(self.0.next())
    }

    /// Pull the next item from the multicast response stream (async).
    pub async fn next_async(&self) -> MulticastStreamMessage {
        to_ffi_message(self.0.next_async().await)
    }
}

/// Bidirectional stream handler for multicast stream-to-unary and
/// stream-to-stream RPC calls.
///
/// Thin wrapper over [`slim_rpc::MulticastBidiStreamHandler`]. Send/close
/// simply delegate; receive converts the native source name into the FFI
/// [`crate::Name`].
#[derive(uniffi::Object)]
pub struct MulticastBidiStreamHandler(Arc<slim_rpc::MulticastBidiStreamHandler>);

impl MulticastBidiStreamHandler {
    /// Wrap a `slim_rpc` handler.
    pub(crate) fn new(inner: Arc<slim_rpc::MulticastBidiStreamHandler>) -> Self {
        Self(inner)
    }
}

#[uniffi::export]
impl MulticastBidiStreamHandler {
    /// Send a request message to the stream (blocking).
    pub fn send(&self, data: Vec<u8>) -> Result<(), RpcError> {
        self.0.send(data)
    }

    /// Send a request message to the stream (async).
    pub async fn send_async(&self, data: Vec<u8>) -> Result<(), RpcError> {
        self.0.send_async(data).await
    }

    /// Close the request stream — signals that no more messages will be sent
    /// (blocking).
    pub fn close_send(&self) -> Result<(), RpcError> {
        self.0.close_send()
    }

    /// Close the request stream (async).
    pub async fn close_send_async(&self) -> Result<(), RpcError> {
        self.0.close_send_async().await
    }

    /// Receive the next response item (blocking).
    pub fn recv(&self) -> MulticastStreamMessage {
        to_ffi_message(self.0.recv())
    }

    /// Receive the next response item (async).
    pub async fn recv_async(&self) -> MulticastStreamMessage {
        to_ffi_message(self.0.recv_async().await)
    }
}
