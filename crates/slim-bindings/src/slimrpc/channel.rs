// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side RPC channel — UniFFI wrapper.
//!
//! [`Channel`] is a thin newtype over [`slim_rpc::Channel`]. All the RPC logic
//! lives in the `agntcy-slim-rpc` crate; this wrapper only adapts the public
//! UniFFI API: constructors take FFI `crate::App` / `crate::Name` values and
//! unwrap them into the native SLIM types, and the multicast calls wrap the
//! returned readers/handlers so their source names surface as FFI names.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::{
    BidiStreamHandler, Metadata, MulticastBidiStreamHandler, MulticastResponseReader,
    RequestStreamWriter, ResponseStreamReader, RpcError,
};

/// Client-side channel for making RPC calls.
///
/// See [`slim_rpc::Channel`] for the full behaviour. This is a UniFFI-exported
/// newtype wrapper that delegates every call to the inner channel.
#[derive(Clone, uniffi::Object)]
pub struct Channel(slim_rpc::Channel);

#[uniffi::export]
impl Channel {
    #[uniffi::constructor]
    pub fn new(app: Arc<crate::App>, remote: Arc<crate::Name>) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    #[uniffi::constructor]
    pub fn new_with_connection(
        app: Arc<crate::App>,
        remote: Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let slim_app = app.inner_app().clone();
        let slim_name = remote.as_slim_name().clone();
        Self(
            slim_rpc::Channel::new_with_members_internal(
                slim_app,
                vec![slim_name],
                false,
                connection_id,
            )
            .expect("single non-empty member list is always valid"),
        )
    }

    #[uniffi::constructor]
    pub fn new_group(
        app: Arc<crate::App>,
        members: Vec<Arc<crate::Name>>,
    ) -> Result<Self, RpcError> {
        Self::new_group_with_connection(app, members, None)
    }

    #[uniffi::constructor]
    pub fn new_group_with_connection(
        app: Arc<crate::App>,
        members: Vec<Arc<crate::Name>>,
        connection_id: Option<u64>,
    ) -> Result<Self, RpcError> {
        let slim_app = app.inner_app().clone();
        let slim_names = members.iter().map(|n| n.as_slim_name().clone()).collect();
        Ok(Self(slim_rpc::Channel::new_with_members_internal(
            slim_app,
            slim_names,
            true,
            connection_id,
        )?))
    }

    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        self.0
            .call_unary(service_name, method_name, request, timeout, metadata)
    }

    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        self.0
            .call_unary_async(service_name, method_name, request, timeout, metadata)
            .await
    }

    pub fn call_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        self.0
            .call_unary_stream(service_name, method_name, request, timeout, metadata)
    }

    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        self.0
            .call_unary_stream_async(service_name, method_name, request, timeout, metadata)
            .await
    }

    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<RequestStreamWriter> {
        self.0
            .call_stream_unary(service_name, method_name, timeout, metadata)
    }

    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Arc<BidiStreamHandler> {
        self.0
            .call_stream_stream(service_name, method_name, timeout, metadata)
    }

    // ── Multicast UniFFI methods ───────────────────────────────────────────────

    /// Broadcast one request to all GROUP members and collect their responses.
    pub fn call_multicast_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        let inner =
            self.0
                .call_multicast_unary(service_name, method_name, request, timeout, metadata)?;
        Ok(Arc::new(MulticastResponseReader::new(inner)))
    }

    /// Broadcast one request to all GROUP members and collect their responses
    /// (async).
    pub async fn call_multicast_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        let inner = self
            .0
            .call_multicast_unary_async(service_name, method_name, request, timeout, metadata)
            .await?;
        Ok(Arc::new(MulticastResponseReader::new(inner)))
    }

    /// Broadcast one request to all GROUP members and stream their responses
    /// (blocking).
    pub fn call_multicast_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        let inner = self.0.call_multicast_unary_stream(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        )?;
        Ok(Arc::new(MulticastResponseReader::new(inner)))
    }

    /// Broadcast one request to all GROUP members and stream their responses
    /// (async).
    pub async fn call_multicast_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        let inner = self
            .0
            .call_multicast_unary_stream_async(
                service_name,
                method_name,
                request,
                timeout,
                metadata,
            )
            .await?;
        Ok(Arc::new(MulticastResponseReader::new(inner)))
    }

    /// Broadcast a request stream to all GROUP members and collect their
    /// responses.
    pub fn call_multicast_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<MulticastBidiStreamHandler> {
        let inner =
            self.0
                .call_multicast_stream_unary(service_name, method_name, timeout, metadata);
        Arc::new(MulticastBidiStreamHandler::new(inner))
    }

    /// Broadcast a request stream to all GROUP members and stream their
    /// responses.
    pub fn call_multicast_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<MulticastBidiStreamHandler> {
        let inner =
            self.0
                .call_multicast_stream_stream(service_name, method_name, timeout, metadata);
        Arc::new(MulticastBidiStreamHandler::new(inner))
    }

    /// Close the persistent SLIM session held by this channel, if any.
    pub fn close(&self, timeout: Option<Duration>) -> Result<(), RpcError> {
        self.0.close(timeout)
    }

    /// Async version of [`close`](Self::close).
    pub async fn close_async(&self, timeout: Option<Duration>) -> Result<(), RpcError> {
        self.0.close_async(timeout).await
    }
}
