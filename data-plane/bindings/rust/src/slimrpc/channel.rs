// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side RPC channel implementation
//!
//! Provides a Channel type for making RPC calls to remote services.
//! Supports all gRPC streaming patterns over SLIM sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use display_error_chain::ErrorChainExt;
use futures::StreamExt;
use futures::future::join_all;
use futures::stream::Stream;
use futures_timer::Delay;
use parking_lot::RwLock as ParkingRwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;

use super::{
    BidiStreamHandler, Context, METHOD_KEY, Metadata, RPC_ID_KEY, ReceivedMessage,
    RequestStreamWriter, ResponseStreamReader, RpcCode, RpcError, SERVICE_KEY, STATUS_CODE_KEY,
    calculate_deadline, calculate_timeout_duration,
    codec::{Decoder, Encoder},
    msg_is_terminal,
    session_wrapper::{SessionRx, SessionTx, new_session},
};

/// Routes incoming response messages to the correct per-RPC mpsc channel.
///
/// The background `response_dispatcher_task` calls `dispatch()` for each message
/// it receives from the shared `SessionRx`.  RPC callers register before sending
/// their request and unregister after they have received all expected responses.
struct ResponseDispatcher {
    pending: ParkingRwLock<HashMap<String, mpsc::UnboundedSender<ReceivedMessage>>>,
}

impl ResponseDispatcher {
    fn new() -> Self {
        Self {
            pending: ParkingRwLock::new(HashMap::new()),
        }
    }

    /// Register a new RPC call and return the receiver end of its private channel.
    fn register(&self, rpc_id: &str) -> mpsc::UnboundedReceiver<ReceivedMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.write().insert(rpc_id.to_string(), tx);
        rx
    }

    /// Remove the registration for a finished RPC call.
    fn unregister(&self, rpc_id: &str) {
        self.pending.write().remove(rpc_id);
    }

    /// Forward a message to the channel registered for the given `rpc_id`.
    ///
    /// Returns `true` when the message was delivered, `false` when no registration
    /// exists for that id (e.g. the caller already timed out).
    fn dispatch(&self, msg: ReceivedMessage, rpc_id: &str) -> bool {
        let lock = self.pending.read();
        if let Some(tx) = lock.get(rpc_id) {
            tx.send(msg).is_ok()
        } else {
            false
        }
    }
}

/// Reads every message from the shared session and routes it to the per-RPC
/// mpsc channel that matches the `rpc-id` metadata field.
async fn response_dispatcher_task(mut session_rx: SessionRx, dispatcher: Arc<ResponseDispatcher>) {
    loop {
        match session_rx.get_message(None).await {
            Ok(msg) => {
                let rpc_id = msg.metadata.get(RPC_ID_KEY).cloned().unwrap_or_default();
                if !dispatcher.dispatch(msg, &rpc_id) {
                    tracing::warn!(%rpc_id, "Received response for unknown or expired RPC ID");
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "Response dispatcher: session closed");
                break;
            }
        }
    }
}

/// A live SLIM session together with its dispatcher and background task.
struct ChannelSession {
    tx: SessionTx,
    dispatcher: Arc<ResponseDispatcher>,
    /// Background task reading from the session.  When finished the session
    /// is considered dead and will be recreated on the next RPC call.
    task: JoinHandle<()>,
}

impl ChannelSession {
    fn is_alive(&self) -> bool {
        !self.task.is_finished()
    }
}

/// Generate a unique RPC ID
fn generate_rpc_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Metadata for the first (or only) message of an RPC call.
/// Carries routing info (service, method), the rpc-id, and any deadline / user
/// metadata from the context.
fn first_msg_metadata(ctx: &Context, service: &str, method: &str, rpc_id: &str) -> Metadata {
    let mut meta = ctx.metadata();
    meta.insert(SERVICE_KEY.to_string(), service.to_string());
    meta.insert(METHOD_KEY.to_string(), method.to_string());
    meta.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    meta
}

/// Metadata for a stream continuation message (not the first).
/// Only carries the rpc-id so the server demultiplexer can route it.
fn continuation_metadata(rpc_id: &str) -> Metadata {
    let mut meta = HashMap::new();
    meta.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    meta
}

/// Metadata for an end-of-stream marker.
/// Carries status OK + rpc-id. When the stream was empty `routing` provides
/// the context/service/method so the server can identify the handler.
fn eos_metadata(rpc_id: &str, routing: Option<(&Context, &str, &str)>) -> Metadata {
    let mut meta = match routing {
        Some((ctx, service, method)) => first_msg_metadata(ctx, service, method, rpc_id),
        None => continuation_metadata(rpc_id),
    };
    let code: i32 = RpcCode::Ok.into();
    meta.insert(STATUS_CODE_KEY.to_string(), code.to_string());
    meta
}

/// Client-side channel for making RPC calls
///
/// A Channel manages the connection to a remote service and provides methods
/// for making RPC calls with different streaming patterns.
///
/// A single SLIM session is maintained per remote and reused across concurrent
/// RPC calls (see module-level documentation for the demultiplexing design).
#[derive(Clone, uniffi::Object)]
pub struct Channel {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Remote service name
    remote: Name,
    /// Optional connection ID for session creation propagation
    connection_id: Option<u64>,
    /// Runtime handle for spawning tasks (resolved at construction)
    runtime: tokio::runtime::Handle,
    /// Shared persistent session (lazily initialised, recreated when dead)
    session: Arc<ParkingRwLock<Option<ChannelSession>>>,
}

impl Channel {
    /// Create a new channel to a remote service
    pub fn new_internal(app: Arc<SlimApp<AuthProvider, AuthVerifier>>, remote: Name) -> Self {
        Self::new_with_connection_internal(app, remote, None, None)
    }

    /// Create a new channel with optional connection ID for session propagation
    pub fn new_with_connection_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        connection_id: Option<u64>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            app,
            remote,
            connection_id,
            runtime,
            session: Arc::new(ParkingRwLock::new(None)),
        }
    }

    /// Return the existing session if alive, otherwise create a new one.
    ///
    /// Fast path: read lock — return the session if it is alive.
    /// Slow path: create a new session outside any lock, then take the write lock
    /// and store it (re-checking in case a concurrent caller beat us to it).
    async fn get_or_create_session(
        &self,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        // Fast path: cheap read lock, no session creation needed.
        {
            let guard = self.session.read();
            if let Some(ref cs) = *guard
                && cs.is_alive()
            {
                return Ok((cs.tx.clone(), cs.dispatcher.clone()));
            }
        }

        // Slow path: session is absent or dead.
        // Create outside any lock — session creation is async.
        tracing::debug!("no persistent session - recreating");
        let (session_tx, session_rx) = self.create_raw_session().await?;
        let dispatcher = Arc::new(ResponseDispatcher::new());
        let task = self
            .runtime
            .spawn(response_dispatcher_task(session_rx, dispatcher.clone()));
        let new_cs = ChannelSession {
            tx: session_tx.clone(),
            dispatcher: dispatcher.clone(),
            task,
        };

        // Write lock: re-check in case a concurrent caller already recreated.
        let mut guard = self.session.write();
        if let Some(ref existing) = *guard
            && existing.is_alive()
        {
            // Discard ours and return the one that was already there.
            new_cs.task.abort();
            return Ok((existing.tx.clone(), existing.dispatcher.clone()));
        }
        *guard = Some(new_cs);
        Ok((session_tx, dispatcher))
    }

    /// Create a raw SLIM session to the remote peer.
    async fn create_raw_session(&self) -> Result<(SessionTx, SessionRx), RpcError> {
        let app = self.app.clone();
        let remote = self.remote.clone();
        let connection_id = self.connection_id;
        let runtime = &self.runtime;

        let handle = runtime.spawn(async move {
            if let Some(conn_id) = connection_id {
                tracing::debug!(
                    remote = %remote,
                    connection_id = conn_id,
                    "Setting route before creating session"
                );
                if let Err(e) = app.set_route(&remote, conn_id).await {
                    tracing::warn!(
                        remote = %remote,
                        connection_id = conn_id,
                        error = %e,
                        "Failed to set route"
                    );
                }
            }

            tracing::debug!(remote = %remote, "Creating persistent session");

            let slim_config = slim_session::session_config::SessionConfig {
                session_type: ProtoSessionType::PointToPoint,
                mls_enabled: true,
                max_retries: Some(10),
                interval: Some(Duration::from_secs(1)),
                initiator: true,
                metadata: HashMap::new(),
            };

            let (session_ctx, completion) = app
                .create_session(slim_config, remote.clone(), None)
                .await
                .map_err(|e| RpcError::unavailable(format!("Failed to create session: {}", e)))?;

            completion
                .await
                .map_err(|e| RpcError::unavailable(format!("Session handshake failed: {}", e)))?;

            Ok(new_session(session_ctx))
        });

        handle
            .await
            .map_err(|e| RpcError::internal(format!("Session creation task failed: {}", e)))?
    }

    /// Check if received message is end-of-stream or error
    fn check_stream_message(&self, received: &ReceivedMessage) -> Result<Option<()>, RpcError> {
        if msg_is_terminal(received) {
            return Ok(None); // End of stream
        }

        let code = self.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;
        if code != RpcCode::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(RpcError::new(code, message));
        }

        Ok(Some(()))
    }

    /// Parse status code from metadata value
    fn parse_status_code(&self, code_str: Option<&String>) -> Result<RpcCode, RpcError> {
        match code_str {
            Some(s) => s
                .parse::<i32>()
                .ok()
                .and_then(|code| RpcCode::try_from(code).ok())
                .ok_or(RpcError::internal(format!("Invalid status code: {}", s))),
            None => Ok(RpcCode::Ok),
        }
    }

    /// Send a single request message carrying all per-call metadata.
    ///
    /// The first (and only) message of a unary request carries:
    /// - `service` / `method` — handler dispatch
    /// - `rpc-id` — demultiplexing on the shared session
    /// - deadline + user metadata from `ctx`
    async fn send_request<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request: Req,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
    ) -> Result<(), RpcError>
    where
        Req: Encoder,
    {
        let request_bytes = request.encode()?;

        let handle = session
            .publish(
                request_bytes,
                Some("msg".to_string()),
                Some(first_msg_metadata(ctx, service_name, method_name, rpc_id)),
            )
            .await?;

        tracing::debug!(%service_name, %method_name, %rpc_id, "Sent request");
        handle.await.map_err(|e| {
            RpcError::internal(format!(
                "Failed to complete sending request for {}-{}: {}",
                service_name,
                method_name,
                e.chain()
            ))
        })?;

        Ok(())
    }

    /// Send a stream of requests followed by an end-of-stream marker.
    ///
    /// Every message (including the EOS) carries `rpc-id` so the server
    /// demultiplexer can route them to the correct stream handler.
    /// Only the first message carries the service/method/deadline metadata.
    async fn send_request_stream<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
    ) -> Result<(), RpcError>
    where
        Req: Encoder,
    {
        let mut request_stream = std::pin::pin!(request_stream);

        let mut handles = vec![];
        let mut first = true;
        while let Some(request) = request_stream.next().await {
            let request_bytes = request.encode()?;

            let msg_meta = if first {
                first = false;
                first_msg_metadata(ctx, service_name, method_name, rpc_id)
            } else {
                continuation_metadata(rpc_id)
            };

            let handle = session
                .publish(request_bytes, Some("msg".to_string()), Some(msg_meta))
                .await?;
            handles.push(handle);
        }

        // EOS marker — always carries status OK + rpc-id.
        // When the stream was empty, also include service/method/deadline so the
        // server can identify the target handler from this sole message.
        let routing = if first {
            Some((ctx, service_name, method_name))
        } else {
            None
        };
        let handle = session
            .publish(
                Vec::new(),
                Some("msg".to_string()),
                Some(eos_metadata(rpc_id, routing)),
            )
            .await?;
        handles.push(handle);

        let results = join_all(handles).await;
        for result in results {
            result.map_err(|e| {
                RpcError::internal(format!(
                    "Failed to complete sending request for {}-{}: {}",
                    service_name,
                    method_name,
                    ErrorChainExt::chain(&e)
                ))
            })?;
        }

        Ok(())
    }

    /// Receive and decode a single response from an RPC-private mpsc channel.
    ///
    /// Distinguishes between:
    /// - RPC errors: `RpcCode` in metadata from the handler
    /// - Transport errors: channel closed or decode failure
    async fn receive_response_from_channel<Res>(
        &self,
        rx: &mut mpsc::UnboundedReceiver<ReceivedMessage>,
    ) -> Result<Res, ReceiveError>
    where
        Res: Decoder,
    {
        let received = rx.recv().await.ok_or_else(|| {
            ReceiveError::Transport(RpcError::internal("Response channel closed"))
        })?;

        let code = self
            .parse_status_code(received.metadata.get(STATUS_CODE_KEY))
            .map_err(ReceiveError::Transport)?;

        if code != RpcCode::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(ReceiveError::Rpc(RpcError::new(code, message)));
        }

        Res::decode(received.payload).map_err(ReceiveError::Transport)
    }

    /// Make a unary RPC call
    ///
    /// Sends a single request and receives a single response.
    pub async fn unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, RpcError>
    where
        Req: Encoder,
        Res: Decoder,
    {
        tracing::debug!(%service_name, %method_name, "Making unary RPC call");

        let timeout_duration = calculate_timeout_duration(timeout);
        let mut delay = Delay::new(timeout_duration);
        let ctx = self.create_context_for_rpc(timeout, metadata);

        let (session_tx, dispatcher) = tokio::select! {
            result = self.get_or_create_session() => result,
            _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during unary call")),
        }?;

        let rpc_id = generate_rpc_id();
        let mut rx = dispatcher.register(&rpc_id);

        let result = tokio::select! {
            result = async {
                self.send_request(&session_tx, &ctx, request, service_name, method_name, &rpc_id).await?;
                match self.receive_response_from_channel(&mut rx).await {
                    Ok(response) => Ok(response),
                    Err(ReceiveError::Transport(e)) => Err(e),
                    Err(ReceiveError::Rpc(e)) => Err(e),
                }
            } => result,
            _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during unary call")),
        };

        dispatcher.unregister(&rpc_id);
        result
    }

    /// Make a unary-stream RPC call
    ///
    /// Sends a single request and receives a stream of responses.
    pub fn unary_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());

            let (session_tx, dispatcher) = tokio::select! {
                result = channel.get_or_create_session() => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during unary-stream call")),
            }?;

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);

            let send_result = tokio::select! {
                result = channel.send_request(&session_tx, &ctx, request, &service_name, &method_name, &rpc_id) => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded while sending request")),
            };
            if let Err(e) = send_result {
                dispatcher.unregister(&rpc_id);
                Err(e)?;
            }

            loop {
                let received = tokio::select! {
                    msg = rx.recv() => msg.ok_or_else(|| RpcError::internal("Response channel closed")),
                    _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded while receiving stream")),
                };
                let received = match received {
                    Ok(r) => r,
                    Err(e) => {
                        dispatcher.unregister(&rpc_id);
                        Err(e)?
                    }
                };

                if channel.check_stream_message(&received)?.is_none() {
                    dispatcher.unregister(&rpc_id);
                    break;
                }

                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    /// Make a stream-unary RPC call
    ///
    /// Sends a stream of requests and receives a single response.
    pub async fn stream_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, RpcError>
    where
        Req: Encoder,
        Res: Decoder,
    {
        let timeout_duration = calculate_timeout_duration(timeout);
        let mut delay = Delay::new(timeout_duration);
        let ctx = self.create_context_for_rpc(timeout, metadata.clone());

        let (session_tx, dispatcher) = tokio::select! {
            result = self.get_or_create_session() => result,
            _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during stream-unary call")),
        }?;

        let rpc_id = generate_rpc_id();
        let mut rx = dispatcher.register(&rpc_id);

        let result = tokio::select! {
            result = async {
                self.send_request_stream(&session_tx, &ctx, request_stream, service_name, method_name, &rpc_id).await?;
                match self.receive_response_from_channel(&mut rx).await {
                    Ok(response) => Ok(response),
                    Err(ReceiveError::Transport(e)) => Err(e),
                    Err(ReceiveError::Rpc(e)) => Err(e),
                }
            } => result,
            _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during stream-unary call")),
        };

        dispatcher.unregister(&rpc_id);
        result
    }

    /// Make a stream-stream RPC call
    ///
    /// Sends a stream of requests and receives a stream of responses.
    pub fn stream_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());

            let (session_tx, dispatcher) = tokio::select! {
                result = channel.get_or_create_session() => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during stream-stream call")),
            }?;

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);

            // Spawn background task to send requests concurrently with receiving
            let session_tx_for_send = session_tx.clone();
            let ctx_for_send = ctx.clone();
            let service_name_for_send = service_name.clone();
            let method_name_for_send = method_name.clone();
            let rpc_id_for_send = rpc_id.clone();
            let channel_for_send = channel.clone();
            let mut send_handle = channel.runtime.spawn(async move {
                channel_for_send.send_request_stream(
                    &session_tx_for_send,
                    &ctx_for_send,
                    request_stream,
                    &service_name_for_send,
                    &method_name_for_send,
                    &rpc_id_for_send,
                ).await
            });

            let mut send_completed = false;
            loop {
                let received = tokio::select! {
                    msg = rx.recv() => msg.ok_or_else(|| RpcError::internal("Response channel closed")),
                    send_result = &mut send_handle, if !send_completed => {
                        send_completed = true;
                        match send_result {
                            Ok(Ok(_)) => continue,
                            Ok(Err(e)) => {
                                dispatcher.unregister(&rpc_id);
                                Err(e)
                            },
                            Err(e) => {
                                dispatcher.unregister(&rpc_id);
                                Err(RpcError::internal(format!("Send task panicked: {}", e)))
                            },
                        }
                    },
                    _ = &mut delay => {
                        dispatcher.unregister(&rpc_id);
                        Err(RpcError::deadline_exceeded("Client deadline exceeded while receiving stream"))
                    },
                };

                let received = match received {
                    Ok(r) => r,
                    Err(e) => {
                        dispatcher.unregister(&rpc_id);
                        Err(e)?
                    }
                };

                if channel.check_stream_message(&received)?.is_none() {
                    dispatcher.unregister(&rpc_id);
                    break;
                }

                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    /// Create a context for a specific RPC call with deadline
    fn create_context_for_rpc(
        &self,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Context {
        let deadline = calculate_deadline(timeout);
        let mut ctx = Context::new();
        ctx.set_deadline(deadline);
        if let Some(meta) = metadata {
            ctx.metadata_mut().extend(meta);
        }
        ctx
    }

    pub fn app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    pub fn remote(&self) -> &Name {
        &self.remote
    }

    pub fn connection_id(&self) -> Option<u64> {
        self.connection_id
    }
}

/// Result type for receive operations that distinguishes error sources
enum ReceiveError {
    /// Transport-level error (channel closed, decode failure, etc.)
    Transport(RpcError),
    /// RPC-level error from handler (in metadata)
    Rpc(RpcError),
}

#[uniffi::export]
impl Channel {
    /// Create a new RPC channel
    #[uniffi::constructor]
    pub fn new(app: std::sync::Arc<crate::App>, remote: std::sync::Arc<crate::Name>) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    /// Create a new RPC channel with optional connection ID
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: std::sync::Arc<crate::App>,
        remote: std::sync::Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let slim_app = app.inner_app().clone();
        let slim_name = remote.as_slim_name().clone();
        let runtime = crate::get_runtime().handle().clone();

        Self::new_with_connection_internal(slim_app, slim_name, connection_id, Some(runtime))
    }

    /// Make a unary-to-unary RPC call (blocking version)
    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    /// Make a unary-to-unary RPC call (async version)
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

    /// Make a unary-to-stream RPC call (blocking version)
    pub fn call_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    /// Make a unary-to-stream RPC call (async version)
    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        let channel = self.clone();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        crate::get_runtime().spawn(async move {
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

        Ok(std::sync::Arc::new(ResponseStreamReader::new(rx)))
    }

    /// Make a stream-to-unary RPC call (blocking version)
    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> std::sync::Arc<RequestStreamWriter> {
        std::sync::Arc::new(RequestStreamWriter::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    /// Make a stream-to-stream RPC call (blocking version)
    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> std::sync::Arc<BidiStreamHandler> {
        std::sync::Arc::new(BidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status_code() {
        assert_eq!(RpcCode::try_from(0), Ok(RpcCode::Ok));
        assert_eq!(RpcCode::try_from(13), Ok(RpcCode::Internal));
        assert!(RpcCode::try_from(999).is_err());
    }

    #[test]
    fn test_generate_rpc_id() {
        let id1 = generate_rpc_id();
        let id2 = generate_rpc_id();
        assert_eq!(id1.len(), 36);
        assert_eq!(id2.len(), 36);
        assert_ne!(id1, id2);
    }
}
