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

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;

use super::{
    BidiStreamHandler, Code, Context, Metadata, ReceivedMessage, RequestStreamWriter,
    ResponseStreamReader, RpcError, STATUS_CODE_KEY, Status, build_method_subscription_name,
    calculate_deadline, calculate_timeout_duration,
    codec::{Decoder, Encoder},
    session_wrapper::{SessionRx, SessionTx, new_session},
};

/// Client-side channel for making RPC calls
///
/// A Channel manages the connection to a remote service and provides methods
/// for making RPC calls with different streaming patterns.
///
/// Each RPC call creates a new session which is closed after the RPC completes.
#[derive(Clone, uniffi::Object)]
pub struct Channel {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Remote service name (base name, will be extended with service/method)
    remote: Name,
    /// Optional connection ID for session creation propagation
    connection_id: Option<u64>,
    /// Runtime handle for spawning tasks (resolved at construction)
    runtime: tokio::runtime::Handle,
}

impl Channel {
    /// Create a new channel to a remote service
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::Channel;
    /// # use slim_datapath::messages::Name;
    /// # use slim_service::app::App;
    /// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    /// # use std::sync::Arc;
    /// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) {
    /// let remote = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
    /// let channel = Channel::new_internal(app, remote);
    /// # }
    /// ```
    pub fn new_internal(app: Arc<SlimApp<AuthProvider, AuthVerifier>>, remote: Name) -> Self {
        Self::new_with_connection_internal(app, remote, None, None)
    }

    /// Create a new channel with optional connection ID for session propagation
    ///
    /// The connection ID is used to propagate session creation to the next SLIM node,
    /// enabling multi-hop RPC calls.
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    /// * `connection_id` - Optional connection ID for session creation propagation to next SLIM node
    /// * `runtime` - Optional tokio runtime handle for spawning tasks
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::Channel;
    /// # use slim_datapath::messages::Name;
    /// # use slim_service::app::App;
    /// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    /// # use std::sync::Arc;
    /// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) {
    /// let remote = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
    /// let connection_id = Some(12345);
    /// let channel = Channel::new_with_connection_internal(app, remote, connection_id, None);
    /// # }
    /// ```
    pub fn new_with_connection_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        connection_id: Option<u64>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        // Resolve runtime handle: use provided or try to get current
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            app,
            remote,
            connection_id,
            runtime,
        }
    }

    /// Close session and ignore errors
    async fn close_session_safe(
        session: &SessionTx,
        app: &Arc<SlimApp<AuthProvider, AuthVerifier>>,
    ) {
        let _ = session.close(app).await;
    }

    /// Check if received message is end-of-stream or error
    fn check_stream_message(&self, received: &ReceivedMessage) -> Result<Option<()>, Status> {
        let code = self.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;

        // Empty message with OK code signals end of stream
        if code == Code::Ok && received.payload.is_empty() {
            return Ok(None); // End of stream
        }

        if code != Code::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(Status::new(code, message));
        }

        Ok(Some(())) // Continue
    }

    /// Make a unary RPC call
    ///
    /// Sends a single request and receives a single response. This is the simplest
    /// RPC pattern, similar to a regular function call over the network.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service (e.g., "MyService")
    /// * `method_name` - The name of the method (e.g., "GetUser")
    /// * `request` - The request message (must implement `Encoder`)
    /// * `timeout` - Optional timeout duration (defaults to MAX_TIMEOUT)
    /// * `metadata` - Optional metadata to include with the request
    ///
    /// # Returns
    /// The decoded response or a `Status` error
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::{Channel, Status, Encoder, Decoder};
    /// # use std::time::Duration;
    /// # #[derive(Default)]
    /// # struct Request {}
    /// # impl Encoder for Request {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response {}
    /// # impl Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> std::result::Result<(), Status> {
    /// let request = Request::default();
    /// let response: Response = channel.unary(
    ///     "MyService",
    ///     "MyMethod",
    ///     request,
    ///     Some(Duration::from_secs(30)),
    ///     None
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, Status>
    where
        Req: Encoder,
        Res: Decoder,
    {
        tracing::debug!(%service_name, %method_name, "Creating session for unary RPC");

        // Calculate timeout duration
        let timeout_duration = calculate_timeout_duration(timeout);
        let mut delay = Delay::new(timeout_duration);

        // Create context first so we can pass deadline in session metadata
        let ctx = self.create_context_for_rpc(timeout, metadata).await;

        // Wrap the entire operation in a runtime-agnostic timeout using futures-timer
        tokio::select! {
            result = async {
                // Create a new session for this RPC with deadline in metadata
                let (session_tx, mut session_rx) = self.create_session(service_name, method_name, &ctx).await?;

                // Send request
                if let Err(e) = self.send_request(&session_tx, &ctx, request, service_name, method_name).await {
                    // Close session on send error
                    Self::close_session_safe(&session_tx, &self.app).await;
                    return Err(e);
                }

                // Receive and decode response
                let receive_result = self.receive_response(&mut session_rx).await;

                // Close the session after RPC completes
                Self::close_session_safe(&session_tx, &self.app).await;

                match receive_result {
                    Ok(response) => Ok(response),
                    Err(ReceiveError::Transport(status)) => Err(status),
                    Err(ReceiveError::Rpc(status)) => Err(status),
                }
            } => result,
            _ = &mut delay => {
                Err(Status::deadline_exceeded("Client deadline exceeded during unary call"))
            }
        }
    }

    /// Make a unary-stream RPC call
    ///
    /// Sends a single request and receives a stream of responses. Useful for
    /// server-side streaming scenarios like paginated results or real-time updates.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `request` - The request message
    /// * `timeout` - Optional timeout duration
    /// * `metadata` - Optional metadata to include with the request
    ///
    /// # Returns
    /// A stream of decoded responses or `Status` errors
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::{Channel, Status, Encoder, Decoder};
    /// # use futures::{StreamExt, pin_mut};
    /// # #[derive(Default)]
    /// # struct Request { count: i32 }
    /// # impl Encoder for Request {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { value: i32 }
    /// # impl Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> Result<(), Status> {
    /// let request = Request { count: 10 };
    /// let stream = channel.unary_stream::<Request, Response>(
    ///     "MyService",
    ///     "StreamResults",
    ///     request,
    ///     None,
    ///     None
    /// );
    /// pin_mut!(stream);
    ///
    /// while let Some(result) = stream.next().await {
    ///     let response = result?;
    ///     println!("Received: {}", response.value);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn unary_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, Status>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            // Calculate timeout duration
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            // Create context first so we can pass deadline in session metadata
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone()).await;

            // Create a new session for this RPC with timeout using futures-timer
            let session_result = tokio::select! {
                result = channel.create_session(&service_name, &method_name, &ctx) => result,
                _ = &mut delay => {
                    Err(Status::deadline_exceeded("Client deadline exceeded during unary-stream call"))
                }
            };
            let (session_tx, mut session_rx) = session_result?;

            // Send request with timeout using futures-timer
            let send_result = tokio::select! {
                result = channel.send_request(&session_tx, &ctx, request, &service_name, &method_name) => result,
                _ = &mut delay => {
                    Self::close_session_safe(&session_tx, &channel.app).await;
                    Err(Status::deadline_exceeded("Client deadline exceeded while sending request"))
                }
            };
            if let Err(e) = send_result {
                Self::close_session_safe(&session_tx, &channel.app).await;
                Err(e)?;
            }

            // Receive streaming responses with same deadline, yielding as we go
            loop {
                let receive_result = tokio::select! {
                    result = session_rx.get_message(None) => result.map_err(|e| Status::internal(format!("Failed to receive response: {}", e))),
                    _ = &mut delay => {
                        Self::close_session_safe(&session_tx, &channel.app).await;
                        Err(Status::deadline_exceeded("Client deadline exceeded while receiving stream"))
                    }
                };
                let received = match receive_result {
                    Ok(r) => r,
                    Err(e) => {
                        Self::close_session_safe(&session_tx, &channel.app).await;
                        Err(e)?
                    }
                };

                // Check if this is end of stream or an error
                if channel.check_stream_message(&received)?.is_none() {
                    Self::close_session_safe(&session_tx, &channel.app).await;
                    break;
                }

                // Decode and yield response
                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    /// Make a stream-unary RPC call
    ///
    /// Sends a stream of requests and receives a single response. Useful for
    /// client-side streaming scenarios like uploading files or aggregating data.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `request_stream` - A stream of request messages
    /// * `timeout` - Optional timeout duration
    /// * `metadata` - Optional metadata to include with the request
    ///
    /// # Returns
    /// The decoded response or a `Status` error
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::{Channel, Status, Encoder, Decoder};
    /// # use futures::stream;
    /// # #[derive(Default)]
    /// # struct Request { data: Vec<u8> }
    /// # impl Encoder for Request {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { total: usize }
    /// # impl Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> Result<(), Status> {
    /// let requests = vec![
    ///     Request { data: vec![1, 2, 3] },
    ///     Request { data: vec![4, 5, 6] },
    /// ];
    /// let request_stream = stream::iter(requests);
    ///
    /// let response: Response = channel.stream_unary(
    ///     "MyService",
    ///     "Aggregate",
    ///     request_stream,
    ///     None,
    ///     None
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stream_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, Status>
    where
        Req: Encoder,
        Res: Decoder,
    {
        // Calculate timeout duration
        let timeout_duration = calculate_timeout_duration(timeout);
        let mut delay = Delay::new(timeout_duration);

        // Create context first so we can pass deadline in session metadata
        let ctx = self.create_context_for_rpc(timeout, metadata.clone()).await;

        // Wrap the entire operation in a runtime-agnostic timeout using futures-timer
        tokio::select! {
            result = async {
                // Create a new session for this RPC with deadline in metadata
                let (session_tx, mut session_rx) = self.create_session(service_name, method_name, &ctx).await?;

                // Send all requests and end-of-stream marker
                if let Err(e) = self.send_request_stream(&session_tx, &ctx, request_stream, service_name, method_name).await {
                    // Close session on send error
                    Self::close_session_safe(&session_tx, &self.app).await;
                    return Err(e);
                }

                // Receive and decode response
                let receive_result = self.receive_response(&mut session_rx).await;

                // Close the session after RPC completes
                Self::close_session_safe(&session_tx, &self.app).await;

                match receive_result {
                    Ok(response) => Ok(response),
                    Err(ReceiveError::Transport(status)) => Err(status),
                    Err(ReceiveError::Rpc(status)) => Err(status),
                }
            } => result,
            _ = &mut delay => {
                Err(Status::deadline_exceeded("Client deadline exceeded during stream-unary call"))
            }
        }
    }

    /// Make a stream-stream RPC call
    ///
    /// Sends a stream of requests and receives a stream of responses. This is the most
    /// flexible RPC pattern, enabling bidirectional streaming for scenarios like chat
    /// applications or real-time data processing.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `request_stream` - A stream of request messages
    /// * `timeout` - Optional timeout duration
    /// * `metadata` - Optional metadata to include with the request
    ///
    /// # Returns
    /// A stream of decoded responses or `Status` errors
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::{Channel, Status, Encoder, Decoder};
    /// # use futures::{stream, StreamExt, pin_mut};
    /// # #[derive(Default)]
    /// # struct Request { message: String }
    /// # impl Encoder for Request {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { reply: String }
    /// # impl Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> std::result::Result<(), Status> {
    /// let requests = vec![
    ///     Request { message: "hello".to_string() },
    ///     Request { message: "world".to_string() },
    /// ];
    /// let request_stream = stream::iter(requests);
    ///
    /// let response_stream = channel.stream_stream::<Request, Response>(
    ///     "MyService",
    ///     "Chat",
    ///     request_stream,
    ///     None,
    ///     None
    /// );
    /// pin_mut!(response_stream);
    ///
    /// while let Some(result) = response_stream.next().await {
    ///     let response = result?;
    ///     println!("Received: {}", response.reply);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn stream_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, Status>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            // Calculate timeout duration
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            // Create context first so we can pass deadline in session metadata
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone()).await;

            // Create a new session for this RPC with timeout using futures-timer
            let session_result = tokio::select! {
                result = channel.create_session(&service_name, &method_name, &ctx) => result,
                _ = &mut delay => {
                    Err(Status::deadline_exceeded("Client deadline exceeded during stream-stream call"))
                }
            };
            let (session_tx, mut session_rx) = session_result?;

            // Wrap session_tx in Arc to share between send task and main task
            let session_tx = Arc::new(session_tx);

            // Spawn background task to send requests concurrently with receiving
            let session_tx_for_send = session_tx.clone();
            let ctx_for_send = ctx.clone();
            let service_name_for_send = service_name.clone();
            let method_name_for_send = method_name.clone();
            let channel_for_send = channel.clone();
            let mut send_handle = channel.runtime.spawn(async move {
                channel_for_send.send_request_stream(&session_tx_for_send, &ctx_for_send, request_stream, &service_name_for_send, &method_name_for_send).await
            });

            // Receive streaming responses with same deadline, also racing against send completion, yielding as we go
            let mut send_completed = false;
            loop {
                let receive_result = tokio::select! {
                    result = session_rx.get_message(None) => result.map_err(|e| Status::internal(format!("Failed to receive response: {}", e))),
                    send_result = &mut send_handle, if !send_completed => {
                        send_completed = true;
                        // Check if send task completed successfully
                        match send_result {
                            Ok(Ok(_)) => continue, // Send completed successfully, continue receiving
                            Ok(Err(e)) => {
                                Self::close_session_safe(&session_tx, &channel.app).await;
                                Err(e) // Send failed with error
                            },
                            Err(e) => {
                                Self::close_session_safe(&session_tx, &channel.app).await;
                                Err(Status::internal(format!("Send task panicked: {}", e)))
                            },
                        }
                    }
                    _ = &mut delay => {
                        Self::close_session_safe(&session_tx, &channel.app).await;
                        Err(Status::deadline_exceeded("Client deadline exceeded while receiving stream"))
                    },
                };
                let received = match receive_result {
                    Ok(r) => r,
                    Err(e) => {
                        Self::close_session_safe(&session_tx, &channel.app).await;
                        Err(e)?
                    }
                };

                // Check if this is end of stream or an error
                if channel.check_stream_message(&received)?.is_none() {
                    Self::close_session_safe(&session_tx, &channel.app).await;
                    break;
                }

                // Decode and yield response
                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    /// Create a session for an RPC call
    async fn create_session(
        &self,
        service_name: &str,
        method_name: &str,
        ctx: &Context,
    ) -> Result<(SessionTx, SessionRx), Status> {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let ctx = ctx.clone();
        let app = self.app.clone();
        let remote = self.remote.clone();
        let connection_id = self.connection_id;
        let runtime = self.runtime.clone();

        // Spawn the entire session creation in a background task
        let handle = runtime.spawn(async move {
            // Build method-specific subscription name (e.g., org/namespace/app-Service-Method)
            let method_subscription_name =
                build_method_subscription_name(&remote, &service_name, &method_name);

            // Set route if connection_id is provided
            if let Some(conn_id) = connection_id {
                tracing::debug!(
                    %service_name,
                    %method_name,
                    %method_subscription_name,
                    connection_id = conn_id,
                    "Setting route before creating session"
                );

                if let Err(e) = app.set_route(&method_subscription_name, conn_id).await {
                    tracing::warn!(
                        %method_subscription_name,
                        connection_id = conn_id,
                        error = %e,
                        "Failed to set route"
                    );
                }
            }

            // Create the session with optional connection ID for propagation
            tracing::debug!(
                %service_name,
                %method_name,
                %method_subscription_name,
                connection_id = ?connection_id,
                "Creating session"
            );

            // Create session configuration with deadline metadata
            let slim_config = slim_session::session_config::SessionConfig {
                session_type: ProtoSessionType::PointToPoint,
                mls_enabled: false,
                max_retries: Some(3),
                interval: Some(Duration::from_secs(1)),
                initiator: true,
                metadata: ctx.metadata(),
            };

            // Create session to the method-specific subscription name
            let (session_ctx, completion) = app
                .create_session(slim_config, method_subscription_name.clone(), None)
                .await
                .map_err(|e| Status::unavailable(format!("Failed to create session: {}", e)))?;

            // Wait for session handshake completion
            completion
                .await
                .map_err(|e| Status::unavailable(format!("Session handshake failed: {}", e)))?;

            // Wrap the session context for RPC operations
            let (session_tx, session_rx) = new_session(session_ctx);

            Ok((session_tx, session_rx))
        });

        // Await the spawned task
        handle
            .await
            .map_err(|e| Status::internal(format!("Session creation task failed: {}", e)))?
    }

    /// Create a context for a specific RPC call with deadline
    async fn create_context_for_rpc(
        &self,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Context {
        // Calculate deadline for this RPC call
        let deadline = calculate_deadline(timeout);

        // Create a new context with the deadline
        let mut ctx = Context::new();
        ctx.set_deadline(deadline);

        // Merge in user metadata
        if let Some(meta) = metadata {
            ctx.metadata_mut().merge(meta);
        }

        ctx
    }

    /// Parse status code from metadata value
    fn parse_status_code(&self, code_str: Option<&String>) -> Result<Code, Status> {
        match code_str {
            Some(s) => s
                .parse::<i32>()
                .ok()
                .and_then(Code::from_i32)
                .ok_or_else(|| Status::internal(format!("Invalid status code: {}", s))),
            None => Ok(Code::Ok), // Default to OK if not present
        }
    }

    /// Get a reference to the underlying SLIM app
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::Channel;
    /// # fn example(channel: Channel) {
    /// let app = channel.app();
    /// # }
    /// ```
    pub fn app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    /// Get the remote service name
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::Channel;
    /// # fn example(channel: Channel) {
    /// let remote = channel.remote();
    /// println!("Remote: {}", remote);
    /// # }
    /// ```
    pub fn remote(&self) -> &Name {
        &self.remote
    }

    /// Get the optional connection ID used for session propagation
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::Channel;
    /// # fn example(channel: Channel) {
    /// if let Some(conn_id) = channel.connection_id() {
    ///     println!("Connection ID: {}", conn_id);
    /// }
    /// # }
    /// ```
    pub fn connection_id(&self) -> Option<u64> {
        self.connection_id
    }

    /// Send a single request and wait for completion
    async fn send_request<Req>(
        &self,
        session: &SessionTx,
        _ctx: &Context,
        request: Req,
        service_name: &str,
        method_name: &str,
    ) -> Result<(), Status>
    where
        Req: Encoder,
    {
        let request_bytes = request.encode()?;
        let handle = session
            .publish(request_bytes, Some("msg".to_string()), None)
            .await?;

        tracing::debug!(%service_name, %method_name, "Sent request");
        handle.await.map_err(|e| {
            Status::internal(format!(
                "Failed to complete sending request for {}-{}: {}",
                service_name,
                method_name,
                e.chain()
            ))
        })?;

        Ok(())
    }

    /// Send a stream of requests followed by end-of-stream marker
    async fn send_request_stream<Req>(
        &self,
        session: &SessionTx,
        _ctx: &Context,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        service_name: &str,
        method_name: &str,
    ) -> Result<(), Status>
    where
        Req: Encoder,
    {
        // pin the stream in the stack for iteration
        let mut request_stream = std::pin::pin!(request_stream);

        let mut handles = vec![];
        while let Some(request) = request_stream.next().await {
            let request_bytes = request.encode()?;
            let handle = session
                .publish(request_bytes, Some("msg".to_string()), None)
                .await?;
            handles.push(handle);
        }

        // Send end-of-stream marker
        let mut end_metadata = HashMap::new();
        end_metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
        let handle = session
            .publish(Vec::new(), Some("msg".to_string()), Some(end_metadata))
            .await?;
        handles.push(handle);

        // Wait for all requests to be sent
        let results = join_all(handles).await;
        for result in results {
            result.map_err(|e| {
                Status::internal(format!(
                    "Failed to complete sending request for {}-{}: {}",
                    service_name,
                    method_name,
                    ErrorChainExt::chain(&e)
                ))
            })?;
        }

        Ok(())
    }

    /// Receive and decode a single response
    ///
    /// Distinguishes between:
    /// - RPC errors: Status codes in metadata from the handler (keep session open)
    /// - Transport errors: Communication/decoding failures (close session)
    async fn receive_response<Res>(&self, session: &mut SessionRx) -> Result<Res, ReceiveError>
    where
        Res: Decoder,
    {
        // Try to receive message - any error here is a transport error
        let received = session
            .get_message(None)
            .await
            .map_err(ReceiveError::Transport)?;

        // Check status code in metadata - error here means RPC error from handler
        let code = self
            .parse_status_code(received.metadata.get(STATUS_CODE_KEY))
            .map_err(ReceiveError::Transport)?;

        if code != Code::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(ReceiveError::Rpc(Status::new(code, message)));
        }

        // Try to decode response - any error here is a transport error
        let response = Res::decode(received.payload).map_err(ReceiveError::Transport)?;

        Ok(response)
    }
}

/// Result type for receive operations that distinguishes error sources
enum ReceiveError {
    /// Transport-level error (session closed, decode failure, etc.) - should close session
    Transport(Status),
    /// RPC-level error from handler (in metadata) - should keep session open
    Rpc(Status),
}

// UniFFI-compatible methods for foreign language bindings
#[uniffi::export]
impl Channel {
    /// Create a new RPC channel
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance
    /// * `remote` - The remote service name to connect to
    ///
    /// # Returns
    /// A new channel instance
    #[uniffi::constructor]
    pub fn new(app: std::sync::Arc<crate::App>, remote: std::sync::Arc<crate::Name>) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    /// Create a new RPC channel with optional connection ID
    ///
    /// The connection ID is used to set up routing before making RPC calls,
    /// enabling multi-hop RPC calls through specific connections.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance
    /// * `remote` - The remote service name to connect to
    /// * `connection_id` - Optional connection ID for routing setup
    ///
    /// # Returns
    /// A new channel instance
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
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// The response message bytes or an error
    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout,
        ))
    }

    /// Make a unary-to-unary RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// The response message bytes or an error
    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
    ) -> Result<Vec<u8>, RpcError> {
        Ok(self
            .unary(&service_name, &method_name, request, timeout, None)
            .await?)
    }

    /// Make a unary-to-stream RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A stream reader for pulling response messages
    ///
    /// # Note
    /// This returns a ResponseStreamReader that can be used to pull messages
    /// one at a time from the response stream.
    pub fn call_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
        ))
    }

    /// Make a unary-to-stream RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A stream reader for pulling response messages
    ///
    /// # Note
    /// This returns a ResponseStreamReader that can be used to pull messages
    /// one at a time from the response stream.
    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        let channel = self.clone();

        // Create a channel to transfer stream items
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn a task to consume the stream and forward to the channel
        crate::get_runtime().spawn(async move {
            let stream = channel.unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                None,
            );
            futures::pin_mut!(stream);

            while let Some(item) = futures::StreamExt::next(&mut stream).await {
                if tx.send(item).is_err() {
                    // Receiver dropped, stop consuming
                    break;
                }
            }
        });

        Ok(std::sync::Arc::new(ResponseStreamReader::new(rx)))
    }

    /// Make a stream-to-unary RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A RequestStreamWriter for sending request messages and getting the final response
    ///
    /// # Note
    /// This returns a RequestStreamWriter that can be used to send multiple request
    /// messages and then finalize to get the single response.
    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
    ) -> std::sync::Arc<RequestStreamWriter> {
        std::sync::Arc::new(RequestStreamWriter::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
        ))
    }

    /// Make a stream-to-stream RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A BidiStreamHandler for sending and receiving messages
    ///
    /// # Note
    /// This returns a BidiStreamHandler that can be used to send request messages
    /// and read response messages concurrently.
    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
    ) -> std::sync::Arc<BidiStreamHandler> {
        std::sync::Arc::new(BidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status_code() {
        // Test status code parsing
        assert_eq!(Code::from_i32(0), Some(Code::Ok));
        assert_eq!(Code::from_i32(13), Some(Code::Internal));
        assert_eq!(Code::from_i32(999), None);
    }
}
