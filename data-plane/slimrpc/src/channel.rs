// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side RPC channel implementation
//!
//! Provides a Channel type for making RPC calls to remote services.
//! Supports all gRPC streaming patterns over SLIM sessions.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_stream::try_stream;
use display_error_chain::ErrorChainExt;
use futures::StreamExt;
use futures::future::join_all;
use futures::stream::Stream;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;

use crate::{
    Code, Context, MAX_TIMEOUT, Metadata, STATUS_CODE_KEY, Session, Status,
    codec::{Decoder, Encoder},
};

/// Metadata key for RPC service name
const RPC_SERVICE_KEY: &str = "slimrpc-service";

/// Metadata key for RPC method name
const RPC_METHOD_KEY: &str = "slimrpc-method";

/// Client-side channel for making RPC calls
///
/// A Channel manages the connection to a remote service and provides methods
/// for making RPC calls with different streaming patterns.
#[derive(Clone)]
pub struct Channel {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Remote service name (base name, will be extended with service/method)
    remote: Name,
    /// Optional connection ID for session creation propagation
    connection_id: Option<u64>,
}

impl Channel {
    /// Create a new channel
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    pub fn new(app: Arc<SlimApp<AuthProvider, AuthVerifier>>, remote: Name) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    /// Create a new channel with optional connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    /// * `connection_id` - Optional connection ID for session creation propagation to next SLIM node
    pub fn new_with_connection(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        connection_id: Option<u64>,
    ) -> Self {
        Self {
            app,
            remote,
            connection_id,
        }
    }

    /// Make a unary-unary RPC call
    ///
    /// Sends a single request and receives a single response.
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
        tracing::debug!("Creating session for {}-{}", service_name, method_name);

        let (session, ctx) = self
            .create_session(service_name, method_name, timeout, metadata)
            .await?;

        tracing::debug!("Created session for {}-{}", service_name, method_name);

        // Encode and send request
        let request_bytes = request.encode_to_vec()?;
        let handle = session
            .publish(
                request_bytes,
                Some("msg".to_string()),
                Some(ctx.metadata().as_map().clone()),
            )
            .await?;

        tracing::debug!("Sent request for {}-{}", service_name, method_name);
        handle.await.map_err(|e| {
            Status::internal(format!(
                "Failed to complete sending request for {}-{}: {}",
                service_name,
                method_name,
                e.chain().to_string()
            ))
        })?;

        // Receive response
        let received = session.get_message(None).await?;

        // Check status code
        let code = self.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;
        if code != Code::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(Status::new(code, message));
        }

        // Decode response
        let response = Res::decode(&received.payload)?;
        Ok(response)
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
    ) -> impl Stream<Item = Result<Res, Status>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let (session, ctx) = channel
                .create_session(&service_name, &method_name, timeout, metadata)
                .await?;

            // Encode and send request
            let request_bytes = request.encode_to_vec()?;
            let handle = session
                .publish(request_bytes, Some("msg".to_string()), Some(ctx.metadata().as_map().clone()))
                .await?;

            tracing::debug!("Sent request for {}-{}", service_name, method_name);
            handle.await.map_err(|e| {
                Status::internal(format!(
                    "Failed to complete sending request for {}-{}: {}",
                    service_name,
                    method_name,
                    e.chain().to_string()
                ))
            })?;

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
                let response = Res::decode(&received.payload)?;
                yield response;
            }
        }
    }

    /// Make a stream-unary RPC call
    ///
    /// Sends a stream of requests and receives a single response.
    pub async fn stream_unary<Req, Res, S>(
        &self,
        service_name: &str,
        method_name: &str,
        mut request_stream: S,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, Status>
    where
        Req: Encoder,
        Res: Decoder,
        S: Stream<Item = Req> + Unpin,
    {
        let (session, ctx) = self
            .create_session(service_name, method_name, timeout, metadata)
            .await?;

        // Send all requests
        let mut handles = vec![];
        while let Some(request) = request_stream.next().await {
            let request_bytes = request.encode_to_vec()?;
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

        // Receive single response
        let received = session
            .get_message(None)
            .await?;

        // Check status code
        let code = self.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;
        if code != Code::Ok {
            let message = String::from_utf8_lossy(&received.payload).to_string();
            return Err(Status::new(code, message));
        }

        // Decode response
        let response = Res::decode(&received.payload)?;
        Ok(response)
    }

    /// Make a stream-stream RPC call
    ///
    /// Sends a stream of requests and receives a stream of responses.
    pub fn stream_stream<Req, Res, S>(
        &self,
        service_name: &str,
        method_name: &str,
        mut request_stream: S,
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
            let (session, ctx) = channel
                .create_session(&service_name, &method_name, timeout, metadata)
                .await?;

            // Send requests in background task
            let session_bg = session.clone();
            let ctx_clone = ctx.clone();
            tokio::spawn(async move {
                while let Some(request) = request_stream.next().await {
                    if let Ok(request_bytes) = request.encode_to_vec() {
                        let _ = session_bg
                            .publish(request_bytes, Some("msg".to_string()), Some(ctx_clone.metadata().as_map().clone()))
                            .await;
                    }
                }
                // Send end-of-stream marker
                let mut end_metadata = ctx_clone.metadata().as_map().clone();
                end_metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                let _ = session_bg
                    .publish(Vec::new(), Some("msg".to_string()), Some(end_metadata))
                    .await;
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
                let response = Res::decode(&received.payload)?;
                yield response;
            }
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
        let mut session_metadata = metadata
            .as_ref()
            .map(|m| m.as_map().clone())
            .unwrap_or_default();

        // Add service and method to metadata for routing
        session_metadata.insert(RPC_SERVICE_KEY.to_string(), service_name.to_string());
        session_metadata.insert(RPC_METHOD_KEY.to_string(), method_name.to_string());

        // Create the session with optional connection ID for propagation
        tracing::debug!(
            "Creating session for {}/{} to remote: {} with connection_id: {:?}",
            service_name,
            method_name,
            self.remote,
            self.connection_id
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

        // Create session to the server's app name (not method name)
        let (session_ctx, completion) = self
            .app
            .create_session(slim_config, self.remote.clone(), None)
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

    /// Get the underlying app reference
    pub fn app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    /// Get the remote name
    pub fn remote(&self) -> &Name {
        &self.remote
    }

    /// Get the connection ID
    pub fn connection_id(&self) -> Option<u64> {
        self.connection_id
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
