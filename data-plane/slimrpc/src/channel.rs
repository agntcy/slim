// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side RPC channel implementation
//!
//! Provides a Channel type for making RPC calls to remote services.
//! Supports all gRPC streaming patterns over SLIM sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_stream::try_stream;
use display_error_chain::ErrorChainExt;
use futures::StreamExt;
use futures::future::join_all;
use futures::stream::Stream;
use tokio::sync::Mutex;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;

use crate::{
    Code, Context, MAX_TIMEOUT, Metadata, STATUS_CODE_KEY, Session, Status,
    codec::{Decoder, Encoder},
};

/// Cached session entry with lock for serialization
struct CachedSession {
    /// The session instance
    session: Session,
    /// Context for the session
    context: Context,
    /// Mutex to ensure only one RPC at a time
    lock: Arc<Mutex<()>>,
}

impl Clone for CachedSession {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            context: self.context.clone(),
            lock: Arc::clone(&self.lock),
        }
    }
}

/// Inner channel state
struct ChannelInner {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Remote service name (base name, will be extended with service/method)
    remote: Name,
    /// Optional connection ID for session creation propagation
    connection_id: Option<u64>,
    /// Cache of sessions keyed by (service_name, method_name)
    session_cache: Mutex<HashMap<String, CachedSession>>,
}

/// Client-side channel for making RPC calls
///
/// A Channel manages the connection to a remote service and provides methods
/// for making RPC calls with different streaming patterns.
///
/// Sessions are cached and reused for multiple RPC calls to improve performance.
/// Only one RPC can be active on a session at a time (no multiplexing).
#[derive(Clone)]
pub struct Channel {
    inner: Arc<ChannelInner>,
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
    /// # use slim_rpc::Channel;
    /// # use slim_datapath::messages::Name;
    /// # use slim_service::app::App;
    /// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    /// # use std::sync::Arc;
    /// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) {
    /// let remote = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
    /// let channel = Channel::new(app, remote);
    /// # }
    /// ```
    pub fn new(app: Arc<SlimApp<AuthProvider, AuthVerifier>>, remote: Name) -> Self {
        Self::new_with_connection(app, remote, None)
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
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Channel;
    /// # use slim_datapath::messages::Name;
    /// # use slim_service::app::App;
    /// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    /// # use std::sync::Arc;
    /// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) {
    /// let remote = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
    /// let connection_id = Some(42);
    /// let channel = Channel::new_with_connection(app, remote, connection_id);
    /// # }
    /// ```
    pub fn new_with_connection(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        connection_id: Option<u64>,
    ) -> Self {
        Self {
            inner: Arc::new(ChannelInner {
                app,
                remote,
                connection_id,
                session_cache: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Make a unary-unary RPC call
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
    /// # use slim_rpc::{Channel, Status};
    /// # use std::time::Duration;
    /// # #[derive(Default)]
    /// # struct Request {}
    /// # impl slim_rpc::Encoder for Request {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response {}
    /// # impl slim_rpc::Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Response::default()) }
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
        tracing::debug!(%service_name, %method_name, "Getting session");

        let (session, ctx, lock) = self
            .get_or_create_session(service_name, method_name, timeout, metadata)
            .await?;

        // Acquire lock to ensure only one RPC at a time
        let _guard = lock.lock().await;

        tracing::debug!(%service_name, %method_name, "Acquired session lock");

        // Send request
        self.send_request(&session, &ctx, request, service_name, method_name)
            .await?;

        // Receive and decode response
        match self.receive_response(&session).await {
            Ok(response) => Ok(response),
            Err(ReceiveError::Transport(status)) => {
                // Transport error - close session and create new one next time
                self.remove_session_from_cache(service_name, method_name)
                    .await;
                Err(status)
            }
            Err(ReceiveError::Rpc(status)) => {
                // RPC error from handler - keep session open
                Err(status)
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
    /// # use slim_rpc::{Channel, Status};
    /// # use futures::{StreamExt, pin_mut};
    /// # #[derive(Default)]
    /// # struct Request { count: i32 }
    /// # impl slim_rpc::Encoder for Request {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { value: i32 }
    /// # impl slim_rpc::Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> std::result::Result<(), Status> {
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
            let (session, ctx, lock) = channel
                .get_or_create_session(&service_name, &method_name, timeout, metadata)
                .await?;

            // Acquire lock to ensure only one RPC at a time
            let _guard = lock.lock().await;

            // Send request
            channel.send_request(&session, &ctx, request, &service_name, &method_name)
                .await?;

            // Receive streaming responses
            loop {
                let received = session
                    .get_message(None)
                    .await
                    .map_err(|e| Status::internal(format!("Failed to receive response: {}", e)))?;

                // Check status code
                let code = channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;

                // Empty message with OK code signals end of stream
                if code == Code::Ok && received.payload.is_empty() {
                    break;
                }

                if code != Code::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    Err(Status::new(code, message))?;
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
    /// # use slim_rpc::{Channel, Status};
    /// # use futures::stream;
    /// # #[derive(Default)]
    /// # struct Request { data: Vec<u8> }
    /// # impl slim_rpc::Encoder for Request {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { total: usize }
    /// # impl slim_rpc::Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> std::result::Result<(), Status> {
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
    pub async fn stream_unary<Req, Res, S>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: S,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, Status>
    where
        Req: Encoder,
        Res: Decoder,
        S: Stream<Item = Req> + Unpin,
    {
        let (session, ctx, lock) = self
            .get_or_create_session(service_name, method_name, timeout, metadata.clone())
            .await?;

        // Acquire lock to ensure only one RPC at a time
        let _guard = lock.lock().await;

        // Send all requests and end-of-stream marker
        self.send_request_stream(&session, &ctx, request_stream, service_name, method_name)
            .await?;

        // Receive and decode response
        match self.receive_response(&session).await {
            Ok(response) => Ok(response),
            Err(ReceiveError::Transport(status)) => {
                // Transport error - close session and create new one next time
                self.remove_session_from_cache(service_name, method_name)
                    .await;
                Err(status)
            }
            Err(ReceiveError::Rpc(status)) => {
                // RPC error from handler - keep session open
                Err(status)
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
    /// # use slim_rpc::{Channel, Status};
    /// # use futures::{stream, StreamExt, pin_mut};
    /// # #[derive(Default)]
    /// # struct Request { message: String }
    /// # impl slim_rpc::Encoder for Request {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { reply: String }
    /// # impl slim_rpc::Decoder for Response {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Response::default()) }
    /// # }
    /// # async fn example(channel: Channel) -> std::result::Result<(), Status> {
    /// let requests = vec![
    ///     Request { message: "hello".to_string() },
    ///     Request { message: "world".to_string() },
    /// ];
    /// let request_stream = stream::iter(requests);
    ///
    /// let response_stream = channel.stream_stream::<Request, Response, _>(
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
    pub fn stream_stream<Req, Res, S>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: S,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, Status>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
        S: Stream<Item = Req> + Unpin + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let (session, ctx, lock) = channel
                .get_or_create_session(&service_name, &method_name, timeout, metadata)
                .await?;

            // Acquire lock to ensure only one RPC at a time
            let _guard = lock.lock().await;

            // Send requests in background task
            let session_bg = session.clone();
            let ctx_clone = ctx.clone();
            let service_name_clone = service_name.clone();
            let method_name_clone = method_name.clone();
            let channel_bg = channel.clone();
            tokio::spawn(async move {
                let _res = channel_bg.send_request_stream(&session_bg, &ctx_clone, request_stream, &service_name_clone, &method_name_clone).await;
            });

            // Receive streaming responses
            loop {
                let received = session
                    .get_message(None)
                    .await
                    .map_err(|e| Status::internal(format!("Failed to receive response: {}", e)))?;

                // Check status code
                let code = channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;

                // Empty message with OK code signals end of stream
                if code == Code::Ok && received.payload.is_empty() {
                    break;
                }

                if code != Code::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    Err(Status::new(code, message))?;
                }

                // Decode and yield response
                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    /// Get or create a session for an RPC call from the cache
    async fn get_or_create_session(
        &self,
        service_name: &str,
        method_name: &str,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<(Session, Context, Arc<Mutex<()>>), Status> {
        let cache_key = format!("{}/{}", service_name, method_name);

        // Try to get from cache first
        {
            let cache = self.inner.session_cache.lock().await;
            if let Some(cached) = cache.get(&cache_key) {
                tracing::debug!(%service_name, %method_name, "Reusing cached session");
                return Ok((
                    cached.session.clone(),
                    cached.context.clone(),
                    cached.lock.clone(),
                ));
            }
        }

        // Create new session if not in cache
        tracing::debug!(%service_name, %method_name, "Creating new session");
        let (session, ctx) = self
            .create_session(service_name, method_name, timeout, metadata)
            .await?;
        let lock = Arc::new(Mutex::new(()));

        // Store in cache
        {
            let mut cache = self.inner.session_cache.lock().await;
            cache.insert(
                cache_key,
                CachedSession {
                    session: session.clone(),
                    context: ctx.clone(),
                    lock: lock.clone(),
                },
            );
        }

        Ok((session, ctx, lock))
    }

    /// Remove a session from the cache and delete it from the app
    async fn remove_session_from_cache(&self, service_name: &str, method_name: &str) {
        let cache_key = format!("{}/{}", service_name, method_name);
        let mut cache = self.inner.session_cache.lock().await;
        if let Some(cached) = cache.remove(&cache_key) {
            tracing::debug!(%service_name, %method_name, "Removed session from cache");

            // Close the session properly
            let _ = cached.session.close(&self.inner.app).await;
        }
    }

    /// Create a session for an RPC call
    async fn create_session(
        &self,
        service_name: &str,
        method_name: &str,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<(Session, Context), Status> {
        // Create session configuration
        let timeout_duration = timeout.unwrap_or(Duration::from_secs(MAX_TIMEOUT));
        let session_metadata = metadata
            .as_ref()
            .map(|m| m.as_map().clone())
            .unwrap_or_default();

        // Build method-specific subscription name (e.g., org/namespace/app-Service-Method)
        let method_subscription_name =
            crate::build_method_subscription_name(&self.inner.remote, service_name, method_name);

        // Create the session with optional connection ID for propagation
        tracing::debug!(
            %service_name,
            %method_name,
            %method_subscription_name,
            connection_id = ?self.inner.connection_id,
            "Creating session"
        );

        // Create session configuration
        let slim_config = slim_session::session_config::SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            mls_enabled: false,
            max_retries: Some(3),
            interval: Some(Duration::from_secs(1)),
            initiator: true,
            metadata: session_metadata,
        };

        // Create session to the method-specific subscription name
        let (session_ctx, completion) = self
            .inner
            .app
            .create_session(slim_config, method_subscription_name.clone(), None)
            .await
            .map_err(|e| Status::unavailable(format!("Failed to create session: {}", e)))?;

        // Wait for session handshake completion
        completion
            .await
            .map_err(|e| Status::unavailable(format!("Session handshake failed: {}", e)))?;

        // Create context from the session context
        let mut ctx = Context::from_session(&session_ctx);

        // Wrap the session context for RPC operations
        let session = Session::new(session_ctx);

        // Set deadline
        let deadline = SystemTime::now()
            .checked_add(timeout_duration)
            .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(MAX_TIMEOUT));
        ctx.set_deadline(deadline);

        // Merge in user metadata
        if let Some(meta) = metadata {
            ctx.metadata_mut().merge(meta);
        }

        Ok((session, ctx))
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
    /// # use slim_rpc::Channel;
    /// # fn example(channel: Channel) {
    /// let app = channel.app();
    /// # }
    /// ```
    pub fn app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.inner.app
    }

    /// Get the remote service name
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Channel;
    /// # fn example(channel: Channel) {
    /// let remote = channel.remote();
    /// println!("Remote: {}", remote);
    /// # }
    /// ```
    pub fn remote(&self) -> &Name {
        &self.inner.remote
    }

    /// Get the optional connection ID used for session propagation
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Channel;
    /// # fn example(channel: Channel) {
    /// if let Some(conn_id) = channel.connection_id() {
    ///     println!("Connection ID: {}", conn_id);
    /// }
    /// # }
    /// ```
    pub fn connection_id(&self) -> Option<u64> {
        self.inner.connection_id
    }

    /// Close a cached session for a specific service/method
    ///
    /// This removes the session from the cache. The next RPC call will create a new session.
    /// This is useful for explicitly closing a session when you know you won't need it again.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Channel;
    /// # async fn example(channel: Channel) {
    /// // Close the session for a specific method
    /// channel.close_session("MyService", "MyMethod").await;
    /// # }
    /// ```
    pub async fn close_session(&self, service_name: &str, method_name: &str) {
        self.remove_session_from_cache(service_name, method_name)
            .await;
    }

    // Helper methods for common RPC patterns

    /// Send a single request and wait for completion
    async fn send_request<Req>(
        &self,
        session: &Session,
        ctx: &Context,
        request: Req,
        service_name: &str,
        method_name: &str,
    ) -> Result<(), Status>
    where
        Req: Encoder,
    {
        let request_bytes = request.encode()?;
        let handle = session
            .publish(
                request_bytes,
                Some("msg".to_string()),
                Some(ctx.metadata().as_map().clone()),
            )
            .await?;

        tracing::debug!(%service_name, %method_name, "Sent request");
        handle.await.map_err(|e| {
            Status::internal(format!(
                "Failed to complete sending request for {}-{}: {}",
                service_name,
                method_name,
                e.chain().to_string()
            ))
        })?;

        Ok(())
    }

    /// Send a stream of requests followed by end-of-stream marker
    async fn send_request_stream<Req, S>(
        &self,
        session: &Session,
        ctx: &Context,
        mut request_stream: S,
        service_name: &str,
        method_name: &str,
    ) -> Result<(), Status>
    where
        Req: Encoder,
        S: Stream<Item = Req> + Unpin,
    {
        let mut handles = vec![];
        while let Some(request) = request_stream.next().await {
            let request_bytes = request.encode()?;
            let handle = session
                .publish(
                    request_bytes,
                    Some("msg".to_string()),
                    Some(ctx.metadata().as_map().clone()),
                )
                .await?;
            handles.push(handle);
        }

        // Send end-of-stream marker
        let mut end_metadata = ctx.metadata().as_map().clone();
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
                    e.chain().to_string()
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
    async fn receive_response<Res>(&self, session: &Session) -> Result<Res, ReceiveError>
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
