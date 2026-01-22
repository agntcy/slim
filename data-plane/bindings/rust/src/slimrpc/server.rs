// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Server implementation for SlimRPC
//!
//! This module implements async request/response processing for RPC handlers,
//! inspired by Go's goroutine-based server model:
//!
//! ## Server Architecture
//!
//! - **Session Handling**: Each incoming session spawns a tokio task (like Go goroutines)
//! - **Request Streams**: For streaming requests, `spawn_request_reader` runs in a
//!   background task, reading messages from the session and forwarding them via channels
//! - **Response Streams**: Responses are sent asynchronously as they're generated,
//!   without buffering, enabling true streaming
//!
//! ## Handler Processing
//!
//! - **UnaryUnary**: Receive single request, send single response
//! - **UnaryStream**: Receive single request, stream responses as generated
//! - **StreamUnary**: Stream requests via background reader, send single response
//! - **StreamStream**: Stream requests via background reader, stream responses as
//!   generated - fully bidirectional async processing
//!
//! The key difference from the Python implementation is that responses are sent
//! immediately as they're produced, rather than being collected first.

use super::common::{method_to_name, DEADLINE_KEY, MAX_TIMEOUT};
use super::context::{MessageContext, SessionContext};
use super::error::{Result, SRPCError};
use super::rpc::RpcHandler;
use crate::App as BindingsApp;
use futures::StreamExt;
use slim_datapath::messages::Name;
use slim_session::session_controller::SessionController;
use slim_session::{Notification, SessionError};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc::Receiver, RwLock};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ServiceMethod {
    pub service: String,
    pub method: String,
}

impl ServiceMethod {
    pub fn new(service: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            method: method.into(),
        }
    }
}

#[derive(uniffi::Object)]
pub struct RpcServer {
    app: Arc<BindingsApp>,
    notification_rx: Arc<RwLock<Receiver<std::result::Result<Notification, SessionError>>>>,
    handlers: HashMap<ServiceMethod, Arc<RpcHandler>>,
    name_to_handler: HashMap<Name, Arc<RpcHandler>>,
    conn_id: u64,
}

#[uniffi::export]
impl RpcServer {
    /// Create a new RPC server
    #[uniffi::constructor]
    pub fn new(app: Arc<BindingsApp>, conn_id: u64) -> Arc<Self> {
        let notification_rx = app.notification_receiver();
        
        Arc::new(Self {
            app,
            notification_rx,
            handlers: HashMap::new(),
            name_to_handler: HashMap::new(),
            conn_id,
        })
    }
    
    /// Run server (blocking) - NOTE: Consumes self
    pub fn run(self: Arc<Self>) -> Result<()> {
        crate::get_runtime().block_on(async {
            self.run_async().await
        })
    }
    
    /// Run server (async) - NOTE: Consumes self
    pub async fn run_async(self: Arc<Self>) -> Result<()> {
        // Extract the inner RpcServer by trying to unwrap the Arc
        // If there are other references, this will fail
        let mut server = match Arc::try_unwrap(self) {
            Ok(server) => server,
            Err(_) => return Err(SRPCError::Session("Cannot run server: Arc has multiple references".to_string())),
        };
        
        // Get internal app for operations
        let internal_app = server.app.inner_app();
        let app_name = internal_app.app_name();
        
        info!(
            "Subscribing to {} with conn_id {}",
            app_name,
            server.conn_id
        );

        // Subscribe to local name with connection ID
        internal_app
            .subscribe(app_name, Some(server.conn_id))
            .await
            .map_err(SRPCError::Subscription)?;

        // Subscribe to all handler names
        for (service_method, handler) in &server.handlers {
            let subscription_name = method_to_name(
                app_name,
                &service_method.service,
                &service_method.method,
            )?;

            info!(
                "Subscribing to {} with conn_id {}",
                subscription_name, server.conn_id
            );

            internal_app
                .subscribe(&subscription_name, Some(server.conn_id))
                .await
                .map_err(SRPCError::Subscription)?;

            server.name_to_handler
                .insert(subscription_name, handler.clone());
        }

        info!("Server running, waiting for sessions");

        // Get a mutable reference to the notification receiver
        let mut rx = server.notification_rx.write().await;

        // Wait for notifications
        loop {
            tokio::select! {
                _ = slim_signal::shutdown() => {
                    info!("Shutdown signal received");
                    break;
                }
                notification = rx.recv() => {
                    let notification = match notification {
                        None => {
                            info!("Notification channel closed");
                            break;
                        }
                        Some(res) => match res {
                            Ok(n) => n,
                            Err(e) => {
                                error!("Error receiving notification: {:?}", e);
                                continue;
                            }
                        }
                    };

                    match notification {
                        Notification::NewSession(ctx) => {
                            let session_arc = ctx.session_arc();
                            if session_arc.is_none() {
                                error!("Failed to get session arc");
                                continue;
                            }

                            let session = session_arc.unwrap();
                            info!(
                                "New session created: id={}, from={}, to={}",
                                session.id(),
                                session.dst(),
                                session.source(),
                            );

                            // Normalize the source name by removing the ID component for matching
                            // (handlers are registered with NULL_COMPONENT as ID)
                            let normalized_source = session.source().clone().with_id(Name::NULL_COMPONENT);

                            // Match handler by normalized source
                            let handler = server.name_to_handler.get(&normalized_source);
                            if handler.is_none() {
                                error!(
                                    "No handler found for source: {} (normalized: {}) (available handlers: {:?})",
                                    session.dst(),
                                    normalized_source,
                                    server.name_to_handler.keys().collect::<Vec<_>>()
                                );
                                continue;
                            }

                            let handler = handler.unwrap().clone();
                            let app = server.app.clone();

                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_session(ctx, handler, app).await {
                                    error!("Error handling session: {:?}", e);
                                }
                            });
                        }
                        Notification::NewMessage(_) => {
                            warn!("Received unexpected app-level message");
                        }
                    }
                }
            }
        }

        info!("Server shutting down");
        Ok(())
    }
}

// Internal Rust-only implementation (not exported via UniFFI)
impl RpcServer {
    /// Create a new RPC server (Rust-only, returns owned Server)
    pub fn new_rust(app: Arc<BindingsApp>, conn_id: u64) -> Self {
        let notification_rx = app.notification_receiver();
        
        Self {
            app,
            notification_rx,
            handlers: HashMap::new(),
            name_to_handler: HashMap::new(),
            conn_id,
        }
    }

    /// Register method handlers (Rust-only, requires RpcHandler trait objects)
    pub fn register_method_handlers(
        &mut self,
        service_name: impl Into<String>,
        handlers: HashMap<String, RpcHandler>,
    ) {
        let service = service_name.into();
        for (method_name, handler) in handlers {
            self.register_rpc(&service, &method_name, handler);
        }
    }

    pub fn register_rpc(&mut self, service_name: &str, method_name: &str, handler: RpcHandler) {
        let service_method = ServiceMethod::new(service_name, method_name);
        self.handlers.insert(service_method, Arc::new(handler));
    }

    async fn handle_session(
        session_ctx: slim_session::context::SessionContext,
        handler: Arc<RpcHandler>,
        app: Arc<BindingsApp>,
    ) -> Result<()> {
        let session = session_ctx
            .session_arc()
            .ok_or_else(|| SRPCError::Session("Failed to get session".to_string()))?;

        info!(
            "Handling session {} from {} to {}",
            session.id(),
            session.dst(),
            session.source()
        );

        let session_context = SessionContext::from_session(&session);

        // Get deadline from metadata
        let timeout = session
            .metadata()
            .get(DEADLINE_KEY)
            .and_then(|d| d.parse::<f64>().ok())
            .map(|deadline| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                Duration::from_secs_f64((deadline - now).max(0.0))
            })
            .unwrap_or(Duration::from_secs(MAX_TIMEOUT));

        // Call handler based on type and stream responses asynchronously
        let result = tokio::time::timeout(
            timeout,
            Self::call_handler_streaming(handler.as_ref(), session_ctx, session_context, session.clone()),
        )
        .await;

        match result {
            Ok(Ok(())) => {
                // Success - handler already sent responses
            }
            Ok(Err(e)) => {
                error!("Handler error: {:?}", e);

                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "13".to_string()); // INTERNAL

                session
                    .publish(session.dst(), vec![], None, Some(metadata))
                    .await
                    .ok();

                app.inner_app().delete_session(&session).ok();
            }
            Err(_) => {
                warn!("Session timed out after {:?}", timeout);
                app.inner_app().delete_session(&session).ok();
            }
        }

        Ok(())
    }

    /// Call handler and stream responses asynchronously as they're generated
    async fn call_handler_streaming(
        handler: &RpcHandler,
        mut session_ctx: slim_session::context::SessionContext,
        session_context: SessionContext,
        session: Arc<SessionController>,
    ) -> Result<()> {
        match handler {
            RpcHandler::UnaryUnary(h) => {
                // Get single request
                let msg = session_ctx
                    .rx
                    .recv()
                    .await
                    .ok_or_else(|| SRPCError::Session("Channel closed".to_string()))?
                    .map_err(|e| SRPCError::Session(e.to_string()))?;

                let msg_ctx = MessageContext::from_message(&msg);
                let request = msg
                    .get_payload()
                    .ok_or_else(|| SRPCError::Session("No payload in message".to_string()))?
                    .as_application_payload()
                    .map_err(|e| {
                        SRPCError::Session(format!("Failed to get application payload: {}", e))
                    })?
                    .blob
                    .clone();

                let response = h.call(request, msg_ctx, session_context).await?;
                
                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "0".to_string());
                
                session
                    .publish(session.dst(), response, None, Some(metadata))
                    .await
                    .map_err(SRPCError::PublishError)?;

                Ok(())
            }
            RpcHandler::UnaryStream(h) => {
                // Get single request
                let msg = session_ctx
                    .rx
                    .recv()
                    .await
                    .ok_or_else(|| SRPCError::Session("Channel closed".to_string()))?
                    .map_err(|e| SRPCError::Session(e.to_string()))?;

                let msg_ctx = MessageContext::from_message(&msg);
                let request = msg
                    .get_payload()
                    .ok_or_else(|| SRPCError::Session("No payload in message".to_string()))?
                    .as_application_payload()
                    .map_err(|e| {
                        SRPCError::Session(format!("Failed to get application payload: {}", e))
                    })?
                    .blob
                    .clone();

                let mut response_stream = h.call(request, msg_ctx, session_context).await?;

                // Stream responses asynchronously as they're generated
                while let Some(response) = response_stream.next().await {
                    let response = response?;
                    let mut metadata = HashMap::new();
                    metadata.insert("code".to_string(), "0".to_string());
                    
                    session
                        .publish(session.dst(), response, None, Some(metadata))
                        .await
                        .map_err(SRPCError::PublishError)?;
                }

                // Send end of stream
                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "0".to_string());
                session
                    .publish(session.dst(), vec![], None, Some(metadata))
                    .await
                    .map_err(SRPCError::PublishError)?;

                Ok(())
            }
            RpcHandler::StreamUnary(h) => {
                // Create request stream - pass ownership of rx to the stream
                let (tx, rx_stream) = tokio::sync::mpsc::unbounded_channel();
                let (_session_weak, rx) = session_ctx.into_parts();

                tokio::spawn(Self::spawn_request_reader(rx, tx));

                let request_stream = Box::pin(
                    tokio_stream::wrappers::UnboundedReceiverStream::new(rx_stream),
                );
                let response = h.call(request_stream, session_context).await?;
                
                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "0".to_string());
                
                session
                    .publish(session.dst(), response, None, Some(metadata))
                    .await
                    .map_err(SRPCError::PublishError)?;

                Ok(())
            }
            RpcHandler::StreamStream(h) => {
                // Create request stream - pass ownership of rx to the stream
                let (tx, rx_stream) = tokio::sync::mpsc::unbounded_channel();
                let (_session_weak, rx) = session_ctx.into_parts();

                tokio::spawn(Self::spawn_request_reader(rx, tx));

                let request_stream = Box::pin(
                    tokio_stream::wrappers::UnboundedReceiverStream::new(rx_stream),
                );
                let mut response_stream = h.call(request_stream, session_context).await?;

                // Stream responses asynchronously as they're generated
                while let Some(response) = response_stream.next().await {
                    let response = response?;
                    let mut metadata = HashMap::new();
                    metadata.insert("code".to_string(), "0".to_string());
                    
                    session
                        .publish(session.dst(), response, None, Some(metadata))
                        .await
                        .map_err(SRPCError::PublishError)?;
                }

                // Send end of stream
                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "0".to_string());
                session
                    .publish(session.dst(), vec![], None, Some(metadata))
                    .await
                    .map_err(SRPCError::PublishError)?;

                Ok(())
            }
        }
    }

    async fn spawn_request_reader(
        mut rx: slim_session::AppChannelReceiver,
        tx: tokio::sync::mpsc::UnboundedSender<Result<(Vec<u8>, MessageContext)>>,
    ) {
        loop {
            match rx.recv().await {
                Some(Ok(msg)) => {
                    let msg_ctx = MessageContext::from_message(&msg);

                    // Check for end of stream
                    let metadata = msg.get_metadata_map();
                    if metadata.get("code") == Some(&"0".to_string()) && msg.get_payload().is_none()
                    {
                        break;
                    }

                    let request = msg
                        .get_payload()
                        .ok_or_else(|| SRPCError::Session("No payload in message".to_string()))
                        .and_then(|p| {
                            p.as_application_payload()
                                .map(|app_payload| app_payload.blob.clone())
                                .map_err(|e| {
                                    SRPCError::Session(format!(
                                        "Failed to get application payload: {}",
                                        e
                                    ))
                                })
                        });

                    match request {
                        Ok(req) => {
                            if tx.send(Ok((req, msg_ctx))).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
                Some(Err(e)) => {
                    let _ = tx.send(Err(SRPCError::Session(e.to_string())));
                    break;
                }
                None => break,
            }
        }
    }
}
