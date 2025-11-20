// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # UniFFI Bindings Layer
//!
//! This module provides a language-agnostic FFI interface to SLIM.
//! It acts as an adapter between UniFFI's requirements and SLIM's internal API.
//!
//! ## Why a separate wrapper?
//! 1. Eliminates generics (UniFFI doesn't support them well)
//! 2. Manages Tokio runtime for blocking FFI calls
//! 3. Provides stable API independent of internal changes
//! 4. Keeps UniFFI dependencies isolated to bindings crate

use std::sync::Arc;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name as SlimName;
use slim_session::SessionConfig as SlimSessionConfig;

use crate::bindings::adapter::BindingsAdapter;
use crate::bindings::service_ref::ServiceRef;
use crate::errors::ServiceError;

// Re-export uniffi for proc macros
use uniffi;

/// Initialize the crypto provider
#[uniffi::export]
pub fn initialize_crypto() {
    // Crypto initialization happens automatically in slim_auth
    // This is a no-op for now but can be expanded if needed
}

/// Get the version of the SLIM bindings
#[uniffi::export]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Name type for SLIM (Secure Low-Latency Interactive Messaging)
#[derive(uniffi::Record)]
pub struct Name {
    pub components: Vec<String>,
    pub id: Option<u64>,
}

impl From<Name> for SlimName {
    fn from(name: Name) -> Self {
        let components: [String; 3] = [
            name.components.get(0).cloned().unwrap_or_default(),
            name.components.get(1).cloned().unwrap_or_default(),
            name.components.get(2).cloned().unwrap_or_default(),
        ];
        let mut slim_name = SlimName::from_strings(components);
        if let Some(id) = name.id {
            slim_name = slim_name.with_id(id);
        }
        slim_name
    }
}

impl From<&SlimName> for Name {
    fn from(name: &SlimName) -> Self {
        Name {
            components: name.components_strings().iter().map(|s| s.to_string()).collect(),
            id: Some(name.id()),
        }
    }
}

/// Session type enum
#[derive(uniffi::Enum)]
pub enum SessionType {
    PointToPoint,
    Multicast,
}

/// Session configuration
#[derive(uniffi::Record)]
pub struct SessionConfig {
    pub session_type: SessionType,
    pub enable_mls: bool,
}

impl From<SessionConfig> for SlimSessionConfig {
    fn from(config: SessionConfig) -> Self {
        SlimSessionConfig {
            session_type: match config.session_type {
                SessionType::PointToPoint => ProtoSessionType::PointToPoint,
                SessionType::Multicast => ProtoSessionType::Multicast,
            },
            initiator: true,
            ..Default::default()
        }
    }
}

/// Error types for SLIM operations
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SlimError {
    #[error("Configuration error: {message}")]
    ConfigError { message: String },
    #[error("Session error: {message}")]
    SessionError { message: String },
    #[error("Receive error: {message}")]
    ReceiveError { message: String },
    #[error("Send error: {message}")]
    SendError { message: String },
    #[error("Authentication error: {message}")]
    AuthError { message: String },
    #[error("Operation timed out")]
    Timeout,
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Internal error: {message}")]
    InternalError { message: String },
}

impl From<ServiceError> for SlimError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::ConfigError(msg) => SlimError::ConfigError { message: msg },
            ServiceError::ReceiveError(msg) => SlimError::ReceiveError { message: msg },
            ServiceError::SessionError(msg) => SlimError::SessionError { message: msg },
            _ => SlimError::InternalError {
                message: err.to_string(),
            },
        }
    }
}

impl From<slim_session::SessionError> for SlimError {
    fn from(err: slim_session::SessionError) -> Self {
        SlimError::SessionError {
            message: err.to_string(),
        }
    }
}

/// Service manages the SLIM service lifecycle
#[derive(uniffi::Object)]
pub struct Service {
    runtime: Arc<tokio::runtime::Runtime>,
}

#[uniffi::export]
impl Service {
    /// Create a new Service instance
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, SlimError> {
        // Initialize crypto
        initialize_crypto();
        
        // Create a Tokio runtime for internal operations that need it
        // (BindingsAdapter::new internally spawns tasks)
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SlimError::InternalError {
                message: format!("Failed to create Tokio runtime: {}", e),
            })?;
        
        Ok(Arc::new(Self {
            runtime: Arc::new(runtime),
        }))
    }
    
    /// Create an app with the given name and shared secret
    pub fn create_app(&self, app_name: Name, shared_secret: String) -> Result<Arc<App>, SlimError> {
        let slim_name: SlimName = app_name.into();
        let provider = SharedSecret::new(
            &slim_name.components_strings()[1], // Use app component as app name
            &shared_secret,
        );
        let verifier = provider.clone();
        
        // BindingsAdapter::new() is synchronous but internally uses Tokio primitives
        // We need to call it within a runtime context using spawn_blocking
        let (adapter, service_ref) = self.runtime.block_on(async {
            tokio::task::spawn_blocking(move || {
                BindingsAdapter::new(slim_name, provider, verifier, false)
            })
            .await
            .map_err(|e| ServiceError::ConfigError(format!("Task join error: {}", e)))?
        })?;
        
        Ok(Arc::new(App {
            adapter,
            _service_ref: service_ref,
            runtime: Arc::clone(&self.runtime),
        }))
    }
}

/// App represents a SLIM application instance
#[derive(uniffi::Object)]
pub struct App {
    adapter: BindingsAdapter<SharedSecret, SharedSecret>,
    _service_ref: ServiceRef,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[uniffi::export]
impl App {
    /// Get the app ID
    pub fn id(&self) -> u64 {
        self.adapter.id()
    }
    
    /// Get the app name
    pub fn name(&self) -> Name {
        Name::from(self.adapter.name())
    }
    
    /// Create a new session
    pub fn create_session(
        &self,
        config: SessionConfig,
        destination: Name,
    ) -> Result<Arc<SessionContext>, SlimError> {
        let slim_config: SlimSessionConfig = config.into();
        let slim_dest: SlimName = destination.into();
        
        let adapter = &self.adapter;
        let (session_ctx, _completion) = self.runtime.block_on(async {
            adapter.create_session(slim_config, slim_dest).await
        })?;
        
        // Convert SessionContext to BindingsSessionContext
        let bindings_ctx = crate::bindings::BindingsSessionContext::from(session_ctx);
        
        Ok(Arc::new(SessionContext {
            inner: bindings_ctx,
            runtime: Arc::clone(&self.runtime),
        }))
    }
    
    /// Delete a session
    pub fn delete_session(&self, session: Arc<SessionContext>) -> Result<(), SlimError> {
        let session_ref = session.inner.session.upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;
        
        self.adapter
            .delete_session(&session_ref)
            .map(|_| ())
            .map_err(|e| SlimError::SessionError {
                message: format!("Failed to delete session: {}", e),
            })
    }
    
    /// Subscribe to a name
    pub fn subscribe(&self, name: Name, connection_id: Option<u64>) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        let adapter = &self.adapter;
        self.runtime.block_on(async {
            adapter.subscribe(&slim_name, connection_id).await
        })?;
        Ok(())
    }
    
    /// Unsubscribe from a name
    pub fn unsubscribe(&self, name: Name, connection_id: Option<u64>) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        let adapter = &self.adapter;
        self.runtime.block_on(async {
            adapter.unsubscribe(&slim_name, connection_id).await
        })?;
        Ok(())
    }
    
    /// Set a route to a name for a specific connection
    pub fn set_route(&self, name: Name, connection_id: u64) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        let adapter = &self.adapter;
        self.runtime.block_on(async {
            adapter.set_route(&slim_name, connection_id).await
        })?;
        Ok(())
    }
    
    /// Remove a route
    pub fn remove_route(&self, name: Name, connection_id: u64) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        let adapter = &self.adapter;
        self.runtime.block_on(async {
            adapter.remove_route(&slim_name, connection_id).await
        })?;
        Ok(())
    }
    
    /// Listen for incoming sessions
    pub fn listen_for_session(&self, timeout_ms: Option<u32>) -> Result<Arc<SessionContext>, SlimError> {
        let timeout = timeout_ms.map(|ms| std::time::Duration::from_millis(ms as u64));
        
        let adapter = &self.adapter;
        let session_ctx = self.runtime.block_on(async {
            adapter.listen_for_session(timeout).await
        })?;
        
        // Convert SessionContext to BindingsSessionContext
        let bindings_ctx = crate::bindings::BindingsSessionContext::from(session_ctx);
        
        Ok(Arc::new(SessionContext {
            inner: bindings_ctx,
            runtime: Arc::clone(&self.runtime),
        }))
    }
}

/// SessionContext represents an active session
#[derive(uniffi::Object)]
pub struct SessionContext {
    inner: crate::bindings::BindingsSessionContext,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[uniffi::export]
impl SessionContext {
    /// Publish a message to the session
    pub fn publish(
        &self,
        destination: Name,
        fanout: u32,
        data: Vec<u8>,
        connection_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        let slim_dest: SlimName = destination.into();
        let inner = &self.inner;
        
        self.runtime.block_on(async {
            inner
                .publish(&slim_dest, fanout, data, connection_out, payload_type, metadata)
                .await
        })
        .map(|_| ())
        .map_err(|e| SlimError::SendError {
            message: e.to_string(),
        })
    }
    
    /// Invite a participant to the session
    pub fn invite(&self, participant: Name) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.into();
        let inner = &self.inner;
        
        self.runtime.block_on(async {
            inner.invite(&slim_name).await
        })
        .map(|_| ())
        .map_err(|e| SlimError::SessionError {
            message: e.to_string(),
        })
    }
    
    /// Remove a participant from the session
    pub fn remove(&self, participant: Name) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.into();
        let inner = &self.inner;
        
        self.runtime.block_on(async {
            inner.remove(&slim_name).await
        })
        .map(|_| ())
        .map_err(|e| SlimError::SessionError {
            message: e.to_string(),
        })
    }
}

