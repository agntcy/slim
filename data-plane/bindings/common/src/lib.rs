// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SLIM Go Bindings using UniFFI
//!
//! This crate provides Go language bindings for the SLIM (Secure Low-Latency 
//! Interactive Messaging) system using Mozilla's UniFFI framework.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slim_auth::shared_secret::SharedSecret;
use slim_config::component::ComponentBuilder;
use slim_datapath::messages::Name as SlimName;
use slim_service::bindings::{
    BindingsAdapter, BindingsSessionContext, MessageContext as SlimMessageContext,
};
use slim_service::Service as SlimService;
use slim_session::SessionConfig as SlimSessionConfig;

mod error;
use error::{Result, SlimError};

// Include the scaffolding generated from the UDL file
uniffi::include_scaffolding!("slim_bindings");

// ============================================================================
// Helper Functions
// ============================================================================

/// Initialize the crypto provider (must be called before using the library)
pub fn initialize_crypto() {
    slim_config::tls::provider::initialize_crypto_provider();
}

/// Get the version of the SLIM bindings
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ============================================================================
// Name Type
// ============================================================================

/// Hierarchical name for identity
#[derive(Debug, Clone)]
pub struct Name {
    pub components: Vec<String>,
    pub id: Option<u64>,
}

impl From<SlimName> for Name {
    fn from(name: SlimName) -> Self {
        Name {
            components: name.components_strings().iter().map(|s| s.to_string()).collect(),
            id: Some(name.id()),
        }
    }
}

impl From<Name> for SlimName {
    fn from(name: Name) -> Self {
        // Convert Vec<String> to array [String; 3]
        let components: [String; 3] = [
            name.components.get(0).cloned().unwrap_or_default(),
            name.components.get(1).cloned().unwrap_or_default(),
            name.components.get(2).cloned().unwrap_or_default(),
        ];
        let slim_name = SlimName::from_strings(components);
        if let Some(id) = name.id {
            slim_name.with_id(id)
        } else {
            slim_name
        }
    }
}

// ============================================================================
// Message Context
// ============================================================================

/// Context for a received message
#[derive(Debug, Clone)]
pub struct MessageContext {
    pub source_name: Name,
    pub destination_name: Option<Name>,
    pub payload_type: String,
    pub metadata: HashMap<String, String>,
    pub input_connection: u64,
    pub identity: String,
}

impl From<SlimMessageContext> for MessageContext {
    fn from(ctx: SlimMessageContext) -> Self {
        MessageContext {
            source_name: ctx.source_name.into(),
            destination_name: ctx.destination_name.map(|n| n.into()),
            payload_type: ctx.payload_type,
            metadata: ctx.metadata,
            input_connection: ctx.input_connection,
            identity: ctx.identity,
        }
    }
}

impl From<MessageContext> for SlimMessageContext {
    fn from(ctx: MessageContext) -> Self {
        SlimMessageContext {
            source_name: ctx.source_name.into(),
            destination_name: ctx.destination_name.map(|n| n.into()),
            payload_type: ctx.payload_type,
            metadata: ctx.metadata,
            input_connection: ctx.input_connection,
            identity: ctx.identity,
        }
    }
}

/// Message with context (for receiving)
#[derive(Debug, Clone)]
pub struct MessageWithContext {
    pub context: MessageContext,
    pub payload: Vec<u8>,
}

// ============================================================================
// Session Configuration
// ============================================================================

#[derive(Debug, Clone)]
pub enum SessionType {
    PointToPoint,
    Multicast,
}

impl From<SessionType> for slim_datapath::api::ProtoSessionType {
    fn from(st: SessionType) -> Self {
        match st {
            SessionType::PointToPoint => slim_datapath::api::ProtoSessionType::PointToPoint,
            SessionType::Multicast => slim_datapath::api::ProtoSessionType::Multicast,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub session_type: SessionType,
    pub enable_mls: bool,
}

impl From<SessionConfig> for SlimSessionConfig {
    fn from(config: SessionConfig) -> Self {
        let cfg = SlimSessionConfig::default().with_session_type(config.session_type.into());
        // Note: MLS configuration would be added here if supported
        // For now, enable_mls is ignored as the API doesn't expose this yet
        cfg
    }
}

// ============================================================================
// Service
// ============================================================================

/// SLIM Service - manages the data plane
pub struct Service {
    inner: Arc<SlimService>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl Service {
    /// Create a new SLIM service instance
    pub fn new() -> Result<Self> {
        // Create a Tokio runtime for async operations
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to create runtime: {}", e),
            })?;

        let service = SlimService::builder()
            .build("go-bindings-service".to_string())
            .map_err(|e| SlimError::ConfigError {
                message: e.to_string(),
            })?;

        Ok(Service {
            inner: Arc::new(service),
            runtime: Arc::new(runtime),
        })
    }

    /// Create an app adapter with shared secret authentication
    pub fn create_app(&self, app_name: Name, shared_secret: String) -> Result<Arc<App>> {
        let slim_name: SlimName = app_name.into();
        let provider = SharedSecret::new(&slim_name.to_string(), &shared_secret);
        let verifier = provider.clone();

        // Create the adapter within the runtime context
        // Even though new_with_service is not async, it spawns async tasks
        let service = Arc::clone(&self.inner);
        let adapter = self.runtime.block_on(async {
            BindingsAdapter::new_with_service(&service, slim_name, provider, verifier)
        }).map_err(|e| SlimError::ConfigError {
            message: e.to_string(),
        })?;

        Ok(Arc::new(App {
            adapter: Arc::new(adapter),
            runtime: Arc::clone(&self.runtime),
        }))
    }
}

// ============================================================================
// App
// ============================================================================

type AppAdapter = BindingsAdapter<SharedSecret, SharedSecret>;

/// SLIM App - application-level API
pub struct App {
    adapter: Arc<AppAdapter>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl App {
    /// Get the app's unique ID
    pub fn id(&self) -> u64 {
        self.adapter.id()
    }

    /// Get the app's name
    pub fn name(&self) -> Name {
        self.adapter.name().clone().into()
    }

    /// Create a new session
    pub fn create_session(
        &self,
        config: SessionConfig,
        destination: Name,
    ) -> Result<Arc<SessionContext>> {
        let slim_config: SlimSessionConfig = config.into();
        let slim_destination: SlimName = destination.into();

        // Block on async operation
        let (session_ctx, _completion_handle) = self.runtime.block_on(async {
            self.adapter
                .create_session(slim_config, slim_destination)
                .await
        }).map_err(|e| SlimError::SessionError {
            message: e.to_string(),
        })?;

        Ok(Arc::new(SessionContext {
            inner: Arc::new(BindingsSessionContext::from(session_ctx)),
            runtime: Arc::clone(&self.runtime),
        }))
    }

    /// Delete a session and free its resources
    pub fn delete_session(&self, session: Arc<SessionContext>) -> Result<()> {
        // Try to upgrade the weak reference to the session controller
        let session_ref = session.inner.session.upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        // Delete the session and discard the completion handle
        self.adapter
            .delete_session(&session_ref)
            .map(|_| ()) // Discard the CompletionHandle
            .map_err(|e| SlimError::SessionError {
                message: e.to_string(),
            })
    }

    /// Subscribe to a name
    pub fn subscribe(&self, name: Name, connection_id: Option<u64>) -> Result<()> {
        let slim_name: SlimName = name.into();
        // Block on async operation  
        self.runtime.block_on(async {
            self.adapter
                .subscribe(&slim_name, connection_id)
                .await
        }).map_err(|e| SlimError::InternalError {
            message: e.to_string(),
        })
    }

    /// Unsubscribe from a name
    pub fn unsubscribe(&self, name: Name, connection_id: Option<u64>) -> Result<()> {
        let slim_name: SlimName = name.into();
        // Block on async operation
        self.runtime.block_on(async {
            self.adapter
                .unsubscribe(&slim_name, connection_id)
                .await
        }).map_err(|e| SlimError::InternalError {
            message: e.to_string(),
        })
    }

    /// Set a route to a name
    pub fn set_route(&self, name: Name, connection_id: u64) -> Result<()> {
        let slim_name: SlimName = name.into();
        // Block on async operation
        self.runtime.block_on(async {
            self.adapter
                .set_route(&slim_name, connection_id)
                .await
        }).map_err(|e| SlimError::InternalError {
            message: e.to_string(),
        })
    }

    /// Remove a route to a name
    pub fn remove_route(&self, name: Name, connection_id: u64) -> Result<()> {
        let slim_name: SlimName = name.into();
        // Block on async operation
        self.runtime.block_on(async {
            self.adapter
                .remove_route(&slim_name, connection_id)
                .await
        }).map_err(|e| SlimError::InternalError {
            message: e.to_string(),
        })
    }

    /// Listen for incoming session requests
    pub fn listen_for_session(&self, timeout_ms: Option<u32>) -> Result<Arc<SessionContext>> {
        let timeout = timeout_ms.map(|ms| Duration::from_millis(ms as u64));

        // Block on async operation
        let session_ctx = self.runtime.block_on(async {
            self.adapter
                .listen_for_session(timeout)
                .await
        }).map_err(|e| match e {
                slim_service::ServiceError::ReceiveError(msg) if msg.contains("timed out") => {
                    SlimError::Timeout {
                        message: "Listen for session timed out".to_string(),
                    }
                }
                _ => SlimError::SessionError {
                    message: e.to_string(),
                },
            })?;

        Ok(Arc::new(SessionContext {
            inner: Arc::new(BindingsSessionContext::from(session_ctx)),
            runtime: Arc::clone(&self.runtime),
        }))
    }
}

// ============================================================================
// Session Context
// ============================================================================

/// Session context for messaging
pub struct SessionContext {
    inner: Arc<BindingsSessionContext>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl SessionContext {
    /// Publish a message to a destination
    pub fn publish(
        &self,
        destination: Name,
        fanout: u32,
        payload: Vec<u8>,
        connection_id: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let slim_name: SlimName = destination.into();
        self.runtime.block_on(async {
            self.inner
                .publish(&slim_name, fanout, payload, connection_id, payload_type, metadata)
                .await
        })
            .map(|_| ()) // Discard the CompletionHandle
            .map_err(|e| SlimError::SendError {
                message: e.to_string(),
            })
    }

    /// Reply to a received message
    pub fn publish_to(
        &self,
        original_context: MessageContext,
        payload: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let slim_ctx: SlimMessageContext = original_context.into();
        self.runtime.block_on(async {
            self.inner
                .publish_to(&slim_ctx, payload, payload_type, metadata)
                .await
        })
            .map(|_| ()) // Discard the CompletionHandle
            .map_err(|e| SlimError::SendError {
                message: e.to_string(),
            })
    }

    /// Invite a peer to join the session
    pub fn invite(&self, peer: Name) -> Result<()> {
        let slim_name: SlimName = peer.into();
        self.runtime.block_on(async {
            self.inner
                .invite(&slim_name)
                .await
        })
            .map(|_| ()) // Discard the CompletionHandle
            .map_err(|e| SlimError::SessionError {
                message: e.to_string(),
            })
    }

    /// Remove a peer from the session
    pub fn remove(&self, peer: Name) -> Result<()> {
        let slim_name: SlimName = peer.into();
        self.runtime.block_on(async {
            self.inner
                .remove(&slim_name)
                .await
        })
            .map(|_| ()) // Discard the CompletionHandle
            .map_err(|e| SlimError::SessionError {
                message: e.to_string(),
            })
    }

    /// Receive a message from the session
    pub fn get_message(&self, timeout_ms: Option<u32>) -> Result<MessageWithContext> {
        let timeout = timeout_ms.map(|ms| Duration::from_millis(ms as u64));

        let (ctx, payload) = self.runtime.block_on(async {
            self.inner
                .get_session_message(timeout)
                .await
        }).map_err(|e| match e {
                slim_service::ServiceError::ReceiveError(msg) if msg.contains("timeout") => {
                    SlimError::Timeout {
                        message: "Get message timed out".to_string(),
                    }
                }
                _ => SlimError::ReceiveError {
                    message: e.to_string(),
                },
            })?;

        Ok(MessageWithContext {
            context: ctx.into(),
            payload,
        })
    }
}

