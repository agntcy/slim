// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::common::{method_to_name, DEADLINE_KEY, MAX_TIMEOUT};
use crate::context::{MessageContext, SessionContext};
use crate::error::{Result, SRPCError};
use crate::rpc::{RPCHandler, RPCHandlerType};
use anyhow::Context;
use futures::StreamExt;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;
use slim_service::app::App;
use slim_session::{Notification, SessionError};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Receiver;
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

pub struct Server<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    app: Arc<App<P, V>>,
    notification_rx: Receiver<std::result::Result<Notification, SessionError>>,
    handlers: HashMap<ServiceMethod, Arc<RPCHandler>>,
    name_to_handler: HashMap<Name, Arc<RPCHandler>>,
    conn_id: u64,
}

impl<P, V> Server<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(
        app: App<P, V>,
        notification_rx: Receiver<std::result::Result<Notification, SessionError>>,
        conn_id: u64,
    ) -> Self {
        Self {
            app: Arc::new(app),
            notification_rx,
            handlers: HashMap::new(),
            name_to_handler: HashMap::new(),
            conn_id,
        }
    }

    pub fn register_method_handlers(
        &mut self,
        service_name: impl Into<String>,
        handlers: HashMap<String, RPCHandler>,
    ) {
        let service = service_name.into();
        for (method_name, handler) in handlers {
            self.register_rpc(&service, &method_name, handler);
        }
    }

    pub fn register_rpc(
        &mut self,
        service_name: &str,
        method_name: &str,
        handler: RPCHandler,
    ) {
        let service_method = ServiceMethod::new(service_name, method_name);
        self.handlers.insert(service_method, Arc::new(handler));
    }

    pub async fn run(mut self) -> Result<()> {
        info!("Subscribing to {} with conn_id {}", self.app.app_name(), self.conn_id);

        // Subscribe to local name with connection ID
        self.app
            .subscribe(self.app.app_name(), Some(self.conn_id))
            .await
            .context("Failed to subscribe to local name")?;

        // Subscribe to all handler names
        for (service_method, handler) in &self.handlers {
            let subscription_name = method_to_name(
                self.app.app_name(),
                &service_method.service,
                &service_method.method,
            )
            .context("Failed to create subscription name")?;

            info!("Subscribing to {} with conn_id {}", subscription_name, self.conn_id);
            
            self.app
                .subscribe(&subscription_name, Some(self.conn_id))
                .await
                .context("Failed to subscribe to handler")?;

            self.name_to_handler
                .insert(subscription_name, handler.clone());
        }

        info!("Server running, waiting for sessions");

        // Wait for notifications
        loop {
            tokio::select! {
                _ = slim_signal::shutdown() => {
                    info!("Shutdown signal received");
                    break;
                }
                notification = self.notification_rx.recv() => {
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
                            let handler = self.name_to_handler.get(&normalized_source);
                            if handler.is_none() {
                                error!(
                                    "No handler found for source: {} (normalized: {}) (available handlers: {:?})",
                                    session.dst(),
                                    normalized_source,
                                    self.name_to_handler.keys().collect::<Vec<_>>()
                                );
                                continue;
                            }
                            
                            let handler = handler.unwrap().clone();
                            let app = self.app.clone();
                            
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

    async fn handle_session(
        session_ctx: slim_session::context::SessionContext,
        handler: Arc<RPCHandler>,
        app: Arc<App<P, V>>,
    ) -> Result<()> {
        let session = session_ctx.session_arc().ok_or_else(|| {
            SRPCError::Session("Failed to get session".to_string())
        })?;

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

        // Call handler based on type
        let result = tokio::time::timeout(
            timeout,
            Self::call_handler(handler.as_ref(), session_ctx, session_context),
        )
        .await;

        match result {
            Ok(Ok(responses)) => {
                // Send responses
                for response in responses {
                    let mut metadata = HashMap::new();
                    metadata.insert("code".to_string(), "0".to_string()); // OK
                    
                    session
                        .publish(session.dst(), response, None, Some(metadata))
                        .await
                        .context("Failed to publish response")?;
                }

                // Send end of stream for streaming responses
                if matches!(
                    handler.handler_type(),
                    RPCHandlerType::UnaryStream | RPCHandlerType::StreamStream
                ) {
                    let mut metadata = HashMap::new();
                    metadata.insert("code".to_string(), "0".to_string());
                    
                    session
                        .publish(session.dst(), vec![], None, Some(metadata))
                        .await
                        .context("Failed to send end of stream")?;
                }
            }
            Ok(Err(e)) => {
                error!("Handler error: {:?}", e);
                
                let mut metadata = HashMap::new();
                metadata.insert("code".to_string(), "13".to_string()); // INTERNAL
                
                session
                    .publish(session.dst(), vec![], None, Some(metadata))
                    .await
                    .ok();

                app.delete_session(&session).ok();
            }
            Err(_) => {
                warn!("Session timed out after {:?}", timeout);
                app.delete_session(&session).ok();
            }
        }

        Ok(())
    }

    async fn call_handler(
        handler: &RPCHandler,
        mut session_ctx: slim_session::context::SessionContext,
        session_context: SessionContext,
    ) -> Result<Vec<Vec<u8>>> {
        let _session = session_ctx.session_arc().ok_or_else(|| {
            SRPCError::Session("Failed to get session".to_string())
        })?;

        match handler {
            RPCHandler::UnaryUnary(h) => {
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
                    .map_err(|e| SRPCError::Session(format!("Failed to get application payload: {}", e)))?
                    .blob.clone();

                let response = h.call(request, msg_ctx, session_context).await?;
                Ok(vec![response])
            }
            RPCHandler::UnaryStream(h) => {
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
                    .map_err(|e| SRPCError::Session(format!("Failed to get application payload: {}", e)))?
                    .blob.clone();

                let mut response_stream = h.call(request, msg_ctx, session_context).await?;
                let mut responses = Vec::new();
                
                while let Some(response) = response_stream.next().await {
                    responses.push(response?);
                }
                
                Ok(responses)
            }
            RPCHandler::StreamUnary(h) => {
                // Create request stream - pass ownership of rx to the stream
                let (tx, rx_stream) = tokio::sync::mpsc::unbounded_channel();
                let (_session_weak, rx) = session_ctx.into_parts();
                
                tokio::spawn(Self::spawn_request_reader(rx, tx));
                
                let request_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx_stream));
                let response = h.call(request_stream, session_context).await?;
                Ok(vec![response])
            }
            RPCHandler::StreamStream(h) => {
                // Create request stream - pass ownership of rx to the stream
                let (tx, rx_stream) = tokio::sync::mpsc::unbounded_channel();
                let (_session_weak, rx) = session_ctx.into_parts();
                
                tokio::spawn(Self::spawn_request_reader(rx, tx));
                
                let request_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx_stream));
                let mut response_stream = h.call(request_stream, session_context).await?;
                let mut responses = Vec::new();
                
                while let Some(response) = response_stream.next().await {
                    responses.push(response?);
                }
                
                Ok(responses)
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
                    if metadata.get("code") == Some(&"0".to_string()) {
                        if msg.get_payload().is_none() {
                            break;
                        }
                    }

                    let request = msg
                        .get_payload()
                        .ok_or_else(|| SRPCError::Session("No payload in message".to_string()))
                        .and_then(|p| {
                            p.as_application_payload()
                                .map(|app_payload| app_payload.blob.clone())
                                .map_err(|e| SRPCError::Session(format!("Failed to get application payload: {}", e)))
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
