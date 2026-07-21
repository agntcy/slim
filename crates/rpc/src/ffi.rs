// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! UniFFI export surface for SlimRPC.
//!
//! Every `#[uniffi::export]` impl — the FFI constructors, the trait-object
//! handler registration, and the blocking convenience wrappers — lives here so
//! the feature-gated FFI code is grouped in one place, separate from the native
//! and shared implementations in `channel.rs`, `server.rs`, and
//! `stream_types.rs`. This whole module is compiled only under the `uniffi`
//! feature (see `lib.rs`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;

use slim_datapath::api::ProtoName as Name;

use crate::{
    BidiStreamHandler, Channel, Context, DecodedStream, Metadata, MulticastBidiStreamHandler,
    MulticastResponseReader, RequestStreamWriter, ResponseSink, ResponseStreamReader, RpcError,
    Server, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler,
    UniffiRequestStream,
};

// ── Channel: FFI constructors ───────────────────────────────────────────────

#[uniffi::export]
impl Channel {
    #[uniffi::constructor]
    pub fn new(app: Arc<slim_bindings::App>, remote: Arc<slim_bindings::Name>) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    #[uniffi::constructor]
    pub fn new_with_connection(
        app: Arc<slim_bindings::App>,
        remote: Arc<slim_bindings::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let slim_app = app.inner_app().clone();
        let slim_name = remote.as_slim_name().clone();
        Self::new_with_members_internal(
            slim_app,
            vec![slim_name],
            false,
            connection_id,
            slim_bindings::get_runtime(),
        )
        .expect("single non-empty member list is always valid")
    }

    #[uniffi::constructor]
    pub fn new_group(
        app: Arc<slim_bindings::App>,
        members: Vec<Arc<slim_bindings::Name>>,
    ) -> Result<Self, RpcError> {
        Self::new_group_with_connection(app, members, None)
    }

    #[uniffi::constructor]
    pub fn new_group_with_connection(
        app: Arc<slim_bindings::App>,
        members: Vec<Arc<slim_bindings::Name>>,
        connection_id: Option<u64>,
    ) -> Result<Self, RpcError> {
        let slim_app = app.inner_app().clone();
        let slim_names: Vec<Name> = members.iter().map(|n| n.as_slim_name().clone()).collect();
        Self::new_with_members_internal(
            slim_app,
            slim_names,
            true,
            connection_id,
            slim_bindings::get_runtime(),
        )
    }
}

// ── Channel: blocking raw-bytes RPC call API ────────────────────────

#[uniffi::export]
impl Channel {
    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        self.runtime.block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        self.unary(&service_name, &method_name, request, timeout, metadata)
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
        self.runtime.block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        let channel = self.clone();
        let (tx, rx) = unbounded_channel();
        self.runtime.spawn(async move {
            let stream = channel.unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                metadata,
            );
            futures::pin_mut!(stream);
            while let Some(item) = futures::StreamExt::next(&mut stream).await {
                if tx.send(item).is_err() {
                    break;
                }
            }
        });
        Ok(Arc::new(ResponseStreamReader::new(rx)))
    }

    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<RequestStreamWriter> {
        Arc::new(RequestStreamWriter::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Arc<BidiStreamHandler> {
        Arc::new(BidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    // ── Multicast UniFFI methods ───────────────────────────────────────────────

    /// Broadcast one request to all GROUP members and collect their responses.
    ///
    /// Returns a reader from which each member's response (wrapped in
    /// `MulticastStreamMessage`) can be pulled one at a time (blocking).
    pub fn call_multicast_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        self.runtime.block_on(self.call_multicast_unary_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
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
        let channel = self.clone();
        let (tx, rx) = unbounded_channel();
        self.runtime.spawn(async move {
            let stream = channel.multicast_unary::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                metadata,
            );
            futures::pin_mut!(stream);
            while let Some(item) = futures::StreamExt::next(&mut stream).await {
                if tx.send(item).is_err() {
                    break;
                }
            }
        });
        Ok(Arc::new(MulticastResponseReader::new(rx)))
    }

    /// Broadcast one request to all GROUP members and stream their responses
    /// (blocking).
    ///
    /// Semantically identical to `call_multicast_unary` at the transport level;
    /// the difference is that each member may send multiple responses before its
    /// EOS, which the server handler determines.
    pub fn call_multicast_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Arc<MulticastResponseReader>, RpcError> {
        self.runtime
            .block_on(self.call_multicast_unary_stream_async(
                service_name,
                method_name,
                request,
                timeout,
                metadata,
            ))
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
        let channel = self.clone();
        let (tx, rx) = unbounded_channel();
        self.runtime.spawn(async move {
            let stream = channel.multicast_unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                metadata,
            );
            futures::pin_mut!(stream);
            while let Some(item) = futures::StreamExt::next(&mut stream).await {
                if tx.send(item).is_err() {
                    break;
                }
            }
        });
        Ok(Arc::new(MulticastResponseReader::new(rx)))
    }

    /// Broadcast a request stream to all GROUP members and collect their
    /// responses.
    ///
    /// Returns a handler that lets you send requests and receive responses
    /// concurrently. Use `send` / `send_async` to push request messages,
    /// `close_send` / `close_send_async` to signal end-of-requests, and
    /// `recv` / `recv_async` to pull response items.
    pub fn call_multicast_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<MulticastBidiStreamHandler> {
        Arc::new(MulticastBidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    /// Broadcast a request stream to all GROUP members and stream their
    /// responses.
    ///
    /// Semantically equivalent to `call_multicast_stream_unary` at the
    /// transport level; the difference (one vs many responses per member) is
    /// determined by the server handler.
    pub fn call_multicast_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Arc<MulticastBidiStreamHandler> {
        Arc::new(MulticastBidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    /// Blocking FFI wrapper for [`Channel::close`].
    pub fn close_blocking(&self, timeout: Option<Duration>) -> Result<(), RpcError> {
        self.runtime.block_on(self.close(timeout))
    }

    /// Async FFI wrapper for [`Channel::close`].
    pub async fn close_async(&self, timeout: Option<Duration>) -> Result<(), RpcError> {
        self.close(timeout).await
    }
}

// ── Server: blocking/async serve & shutdown wrappers ────────────────

#[uniffi::export]
impl Server {
    /// Blocking FFI wrapper for [`Server::serve`].
    pub fn serve_blocking(&self) -> Result<(), RpcError> {
        self.runtime.block_on(self.serve())
    }

    /// Async FFI wrapper for [`Server::serve`].
    pub async fn serve_async(&self) -> Result<(), RpcError> {
        self.serve().await
    }

    /// Blocking FFI wrapper for [`Server::shutdown`].
    pub fn shutdown_blocking(&self) {
        self.runtime.block_on(self.shutdown())
    }

    /// Async FFI wrapper for [`Server::shutdown`].
    pub async fn shutdown_async(&self) {
        self.shutdown().await
    }
}

// ── Server: FFI constructors ────────────────────────────────────────

#[uniffi::export]
impl Server {
    /// Create a new RPC server
    ///
    /// This is the primary constructor for creating an RPC server instance
    /// that can handle incoming RPC requests over SLIM.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance that provides the underlying
    ///   network transport and session management
    /// * `base_name` - The base name for this service (e.g., org.namespace.service).
    ///   This name is used to construct subscription names for RPC methods.
    ///
    /// # Returns
    /// A new RPC server instance wrapped in an Arc for shared ownership
    #[uniffi::constructor]
    pub fn new(app: &Arc<slim_bindings::App>, base_name: Arc<slim_bindings::Name>) -> Self {
        Self::new_with_connection(app, base_name, None)
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// The connection ID is used to set up routing before serving RPC requests,
    /// enabling multi-hop RPC calls through specific connections.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance that provides the underlying
    ///   network transport and session management
    /// * `base_name` - The base name for this service (e.g., org.namespace.service).
    ///   This name is used to construct subscription names for RPC methods.
    /// * `connection_id` - Optional connection ID for routing setup
    ///
    /// # Returns
    /// A new RPC server instance wrapped in an Arc for shared ownership
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: &Arc<slim_bindings::App>,
        base_name: Arc<slim_bindings::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let app_inner = app.inner();
        let rx = app.notification_receiver();

        Self::new_with_shared_rx_and_connection(
            app_inner,
            base_name.as_ref().into(),
            connection_id,
            rx,
            Some(slim_bindings::get_runtime()),
        )
    }
}

// ── Server: trait-object handler registration ───────────────────────

#[uniffi::export]
impl Server {
    pub fn register_unary_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryUnaryHandler>,
    ) {
        let service_clone = service_name.clone();
        let method_clone = method_name.clone();

        tracing::debug!(service = %service_clone, method = %method_clone, "Registering unary-unary handler");

        self.register_unary_unary_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: Context| {
                let handler = handler.clone();
                tracing::debug!(service = %service_clone, method = %method_clone, "Handling unary-unary request");

                async move { handler.handle(request, Arc::new(context)).await }
            },
        );
    }
    pub fn register_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryStreamHandler>,
    ) {
        self.register_unary_stream_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: Context| {
                let handler = handler.clone();

                async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handler
                                .handle(request, Arc::new(context), sink.clone())
                                .await
                            {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };

                    // Detach the task - it will run independently
                    drop(handler_task);

                    // Convert the receiver to a stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
                    Ok(stream)
                }
            },
        );
    }
    pub fn register_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamUnaryHandler>,
    ) {
        self.register_stream_unary_internal(
            &service_name,
            &method_name,
            move |stream: DecodedStream<Vec<u8>>, context: Context| {
                let handler = handler.clone();
                let request_stream = Arc::new(UniffiRequestStream::new(stream));

                async move { handler.handle(request_stream, Arc::new(context)).await }
            },
        );
    }
    pub fn register_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamStreamHandler>,
    ) {
        self.register_stream_stream_internal(
            &service_name,
            &method_name,
            move |stream: DecodedStream<Vec<u8>>, context: Context| {
                let handler = handler.clone();
                let request_stream = Arc::new(UniffiRequestStream::new(stream));

                async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handler
                                .handle(request_stream, Arc::new(context), sink.clone())
                                .await
                            {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };

                    // Detach the task - it will run independently
                    drop(handler_task);

                    // Convert the receiver to a stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
                    Ok(stream)
                }
            },
        );
    }
}
