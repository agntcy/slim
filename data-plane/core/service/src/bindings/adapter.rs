// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SLIM Bindings Adapter (UniFFI-Compatible)
//!
//! This module provides a language-agnostic FFI interface to SLIM using a hybrid approach
//! that exposes both synchronous (blocking) and asynchronous versions of operations.
//!
//! ## Architecture
//! - **Flexible Authentication**: Uses `AuthProvider`/`AuthVerifier` enums supporting multiple auth types
//!   (SharedSecret, JWT, SPIRE, StaticToken) instead of generics (UniFFI requirement)
//! - **Hybrid API**: Both sync (FFI-exposed) and async (internal) methods
//! - **Runtime management**: Manages Tokio runtime for blocking operations

use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{RwLock, mpsc};

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::TokenProvider; // For get_token() and get_id()
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name as SlimName;
use slim_session::SessionConfig as SlimSessionConfig;
use slim_session::session_controller::SessionController;
use slim_session::{Notification, SessionError as SlimSessionError};

use crate::app::App;
use crate::bindings::service_ref::{ServiceRef, get_or_init_global_service};
use crate::errors::ServiceError;
use crate::service::Service;
use slim_config::component::ComponentBuilder;

// Re-export uniffi for proc macros
use uniffi;

// ============================================================================
// UniFFI Type Definitions
// ============================================================================

/// Global Tokio runtime for async operations
static GLOBAL_RUNTIME: OnceLock<Arc<tokio::runtime::Runtime>> = OnceLock::new();

/// Get or initialize the global Tokio runtime
fn get_runtime() -> &'static Arc<tokio::runtime::Runtime> {
    GLOBAL_RUNTIME.get_or_init(|| {
        Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime"),
        )
    })
}

/// Initialize the crypto provider
#[uniffi::export]
pub fn initialize_crypto() {
    // Crypto initialization happens automatically in slim_auth
    // Also initialize the global runtime
    let _ = get_runtime();
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
            name.components.first().cloned().unwrap_or_default(),
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
            components: name
                .components_strings()
                .iter()
                .map(|s| s.to_string())
                .collect(),
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

impl From<SlimSessionError> for SlimError {
    fn from(err: SlimSessionError) -> Self {
        SlimError::SessionError {
            message: err.to_string(),
        }
    }
}

/// TLS configuration for server/client
#[derive(uniffi::Record)]
pub struct TlsConfig {
    pub insecure: bool,
}

/// Server configuration for running a SLIM server
#[derive(uniffi::Record)]
pub struct ServerConfig {
    pub endpoint: String,
    pub tls: TlsConfig,
}

/// Client configuration for connecting to a SLIM server
#[derive(uniffi::Record)]
pub struct ClientConfig {
    pub endpoint: String,
    pub tls: TlsConfig,
}

/// Message context for received messages
#[derive(uniffi::Record)]
pub struct MessageContext {
    pub source_name: Name,
    pub destination_name: Option<Name>,
    pub payload_type: String,
    pub metadata: std::collections::HashMap<String, String>,
    pub input_connection: u64,
    pub identity: String,
}

impl From<crate::bindings::MessageContext> for MessageContext {
    fn from(ctx: crate::bindings::MessageContext) -> Self {
        Self {
            source_name: Name::from(&ctx.source_name),
            destination_name: ctx.destination_name.as_ref().map(Name::from),
            payload_type: ctx.payload_type,
            metadata: ctx.metadata,
            input_connection: ctx.input_connection,
            identity: ctx.identity,
        }
    }
}

/// Received message containing context and payload
#[derive(uniffi::Record)]
pub struct ReceivedMessage {
    pub context: MessageContext,
    pub payload: Vec<u8>,
}

// ============================================================================
// FFI Entry Points
// ============================================================================

/// Create an app with the given name and shared secret (blocking version for FFI)
///
/// This is the main entry point for creating a SLIM application from language bindings.
#[uniffi::export]
pub fn create_app_with_secret(
    app_name: Name,
    shared_secret: String,
) -> Result<Arc<BindingsAdapter>, SlimError> {
    let runtime = get_runtime();
    runtime.block_on(async { create_app_with_secret_async(app_name, shared_secret).await })
}

/// Create an app with the given name and shared secret (async version)
async fn create_app_with_secret_async(
    app_name: Name,
    shared_secret: String,
) -> Result<Arc<BindingsAdapter>, SlimError> {
    let slim_name: SlimName = app_name.into();
    let shared_secret_impl = SharedSecret::new(&slim_name.components_strings()[1], &shared_secret);

    // Wrap in enum types for flexible auth support
    let provider = AuthProvider::SharedSecret(shared_secret_impl.clone());
    let verifier = AuthVerifier::SharedSecret(shared_secret_impl);

    let adapter = tokio::task::spawn_blocking(move || {
        BindingsAdapter::new(slim_name, provider, verifier, false)
    })
    .await
    .map_err(|e| ServiceError::ConfigError(format!("Task join error: {}", e)))??;

    Ok(adapter)
}

/// Adapter that bridges the App API with language-bindings interface
///
/// This adapter uses enum-based auth types (`AuthProvider`/`AuthVerifier`) instead of generics
/// to be compatible with UniFFI, supporting multiple authentication mechanisms (SharedSecret,
/// JWT, SPIRE, StaticToken). It provides both synchronous (blocking) and asynchronous methods
/// for flexibility.
#[derive(uniffi::Object)]
pub struct BindingsAdapter {
    /// The underlying App instance with enum-based auth types (supports SharedSecret, JWT, SPIRE)
    app: Arc<App<AuthProvider, AuthVerifier>>,

    /// Channel receiver for notifications from the app
    notification_rx: Arc<RwLock<mpsc::Receiver<Result<Notification, SlimSessionError>>>>,

    /// Service reference for lifecycle management
    service_ref: ServiceRef,

    /// Tokio runtime for blocking async operations
    runtime: Arc<tokio::runtime::Runtime>,
}

impl BindingsAdapter {
    /// Internal constructor - Create a new BindingsAdapter with complete creation logic
    ///
    /// This is not exposed through UniFFI (associated functions not supported).
    /// Use `create_app_with_secret` for FFI instead.
    ///
    /// Accepts `AuthProvider` and `AuthVerifier` enums, supporting multiple auth types:
    /// - SharedSecret
    /// - JWT (JwtSigner/JwtVerifier)
    /// - SPIRE (SpireIdentityManager)
    /// - StaticToken
    pub fn new(
        base_name: SlimName,
        identity_provider: AuthProvider,
        identity_verifier: AuthVerifier,
        use_local_service: bool,
    ) -> Result<Arc<Self>, SlimError> {
        // Validate token
        let _identity_token =
            identity_provider
                .get_token()
                .map_err(|e| SlimError::ConfigError {
                    message: format!("Failed to get token from provider: {}", e),
                })?;

        // Get ID from token and generate name with token ID
        let token_id = identity_provider
            .get_id()
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to get ID from token: {}", e),
            })?;

        // Use a hash of the token ID to convert to u64 for name generation
        let id_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };
        let app_name = base_name.with_id(id_hash);

        // Create or get service
        let service_ref = if use_local_service {
            let svc = Service::builder()
                .build("local-bindings-service".to_string())
                .map_err(|e| SlimError::ConfigError {
                    message: format!("Failed to create local service: {}", e),
                })?;
            ServiceRef::Local(Box::new(svc))
        } else {
            ServiceRef::Global(get_or_init_global_service())
        };

        // Get service reference for adapter creation
        let service = service_ref.get_service();

        // Create the app
        let (app, rx) = service
            .create_app(&app_name, identity_provider, identity_verifier)
            .map_err(SlimError::from)?;

        let runtime = Arc::clone(get_runtime());

        let adapter = Arc::new(Self {
            app: Arc::new(app),
            notification_rx: Arc::new(RwLock::new(rx)),
            service_ref,
            runtime,
        });

        Ok(adapter)
    }

    /// Test-only constructor - Create a new BindingsAdapter with a provided service
    ///
    /// This method allows tests to pass in their own Service instance for better
    /// control and isolation during testing. Not exposed via FFI.
    ///
    /// # Arguments
    /// * `service` - Pre-configured Service instance to use
    /// * `base_name` - Base name for the app (ID will be generated from token)
    /// * `identity_provider` - Authentication provider (AuthProvider enum)
    /// * `identity_verifier` - Authentication verifier (AuthVerifier enum)
    ///
    /// # Returns
    /// * `Ok(Arc<BindingsAdapter>)` - Successfully created adapter
    /// * `Err(SlimError)` - If creation fails
    #[cfg(test)]
    pub fn new_with_service(
        service: &Service,
        base_name: SlimName,
        identity_provider: AuthProvider,
        identity_verifier: AuthVerifier,
    ) -> Result<Arc<Self>, SlimError> {
        // Validate token
        let _identity_token =
            identity_provider
                .get_token()
                .map_err(|e| SlimError::ConfigError {
                    message: format!("Failed to get token from provider: {}", e),
                })?;

        // Get ID from token and generate name with token ID
        let token_id = identity_provider
            .get_id()
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to get ID from token: {}", e),
            })?;

        // Use a hash of the token ID to convert to u64 for name generation
        let id_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };
        let app_name = base_name.with_id(id_hash);

        // Create the app using the provided service
        let (app, rx) = service
            .create_app(&app_name, identity_provider, identity_verifier)
            .map_err(SlimError::from)?;

        let runtime = Arc::clone(get_runtime());

        // Use a global service reference since we don't own the service
        let service_ref = ServiceRef::Global(get_or_init_global_service());

        let adapter = Arc::new(Self {
            app: Arc::new(app),
            notification_rx: Arc::new(RwLock::new(rx)),
            service_ref,
            runtime,
        });

        Ok(adapter)
    }
}

#[uniffi::export]
impl BindingsAdapter {
    /// Get the app ID (derived from name)
    pub fn id(&self) -> u64 {
        self.app.app_name().id()
    }

    /// Get the app name
    pub fn name(&self) -> Name {
        Name::from(self.app.app_name())
    }

    /// Create a new session (blocking version for FFI)
    pub fn create_session(
        &self,
        config: SessionConfig,
        destination: Name,
    ) -> Result<Arc<FFISessionContext>, SlimError> {
        self.runtime
            .block_on(async { self.create_session_async(config, destination).await })
    }

    /// Create a new session (async version)
    pub async fn create_session_async(
        &self,
        config: SessionConfig,
        destination: Name,
    ) -> Result<Arc<FFISessionContext>, SlimError> {
        let slim_config: SlimSessionConfig = config.into();
        let slim_dest: SlimName = destination.into();

        let (session_ctx, _completion) = self
            .app
            .create_session(slim_config, slim_dest, None)
            .await?;

        // Convert SessionContext to BindingsSessionContext
        let bindings_ctx = crate::bindings::BindingsSessionContext::from(session_ctx);

        Ok(Arc::new(FFISessionContext {
            inner: bindings_ctx,
            runtime: Arc::clone(&self.runtime),
        }))
    }

    /// Delete a session (synchronous - no async version needed)
    pub fn delete_session(&self, session: Arc<FFISessionContext>) -> Result<(), SlimError> {
        let session_ref =
            session
                .inner
                .session
                .upgrade()
                .ok_or_else(|| SlimError::SessionError {
                    message: "Session already closed or dropped".to_string(),
                })?;

        self.app
            .delete_session(&session_ref)
            .map(|_| ())
            .map_err(|e| SlimError::SessionError {
                message: format!("Failed to delete session: {}", e),
            })
    }

    /// Subscribe to a name (blocking version for FFI)
    pub fn subscribe(&self, name: Name, connection_id: Option<u64>) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.subscribe_async(name, connection_id).await })
    }

    /// Subscribe to a name (async version)
    pub async fn subscribe_async(
        &self,
        name: Name,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        self.app.subscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Unsubscribe from a name (blocking version for FFI)
    pub fn unsubscribe(&self, name: Name, connection_id: Option<u64>) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.unsubscribe_async(name, connection_id).await })
    }

    /// Unsubscribe from a name (async version)
    pub async fn unsubscribe_async(
        &self,
        name: Name,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        self.app.unsubscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Set a route to a name for a specific connection (blocking version for FFI)
    pub fn set_route(&self, name: Name, connection_id: u64) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.set_route_async(name, connection_id).await })
    }

    /// Set a route to a name for a specific connection (async version)
    pub async fn set_route_async(&self, name: Name, connection_id: u64) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        self.app.set_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Remove a route (blocking version for FFI)
    pub fn remove_route(&self, name: Name, connection_id: u64) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.remove_route_async(name, connection_id).await })
    }

    /// Remove a route (async version)
    pub async fn remove_route_async(
        &self,
        name: Name,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.into();
        self.app.remove_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Listen for incoming sessions (blocking version for FFI)
    pub fn listen_for_session(
        &self,
        timeout_ms: Option<u32>,
    ) -> Result<Arc<FFISessionContext>, SlimError> {
        self.runtime
            .block_on(async { self.listen_for_session_async(timeout_ms).await })
    }

    /// Listen for incoming sessions (async version)
    pub async fn listen_for_session_async(
        &self,
        timeout_ms: Option<u32>,
    ) -> Result<Arc<FFISessionContext>, SlimError> {
        let timeout = timeout_ms.map(|ms| std::time::Duration::from_millis(ms as u64));

        let mut rx = self.notification_rx.write().await;

        let recv_fut = rx.recv();
        let notification_opt = if let Some(dur) = timeout {
            match tokio::time::timeout(dur, recv_fut).await {
                Ok(n) => n,
                Err(_) => {
                    return Err(SlimError::ReceiveError {
                        message: "listen_for_session timed out".to_string(),
                    });
                }
            }
        } else {
            recv_fut.await
        };

        if notification_opt.is_none() {
            return Err(SlimError::ReceiveError {
                message: "application channel closed".to_string(),
            });
        }

        match notification_opt.unwrap() {
            Ok(Notification::NewSession(ctx)) => {
                let bindings_ctx = crate::bindings::BindingsSessionContext::from(ctx);
                Ok(Arc::new(FFISessionContext {
                    inner: bindings_ctx,
                    runtime: Arc::clone(&self.runtime),
                }))
            }
            Ok(Notification::NewMessage(_)) => Err(SlimError::ReceiveError {
                message: "received unexpected message notification while listening for session"
                    .to_string(),
            }),
            Err(e) => Err(SlimError::ReceiveError {
                message: format!("failed to receive session notification: {}", e),
            }),
        }
    }

    /// Run a SLIM server on the specified endpoint (blocking version for FFI)
    ///
    /// # Arguments
    /// * `config` - Server configuration (endpoint and TLS settings)
    ///
    /// # Returns
    /// * `Ok(())` - Server started successfully
    /// * `Err(SlimError)` - If server startup fails
    pub fn run_server(&self, config: ServerConfig) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.run_server_async(config).await })
    }

    /// Run a SLIM server (async version)
    pub async fn run_server_async(&self, config: ServerConfig) -> Result<(), SlimError> {
        use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
        use slim_config::tls::server::TlsServerConfig;

        let tls_config = TlsServerConfig::new().with_insecure(config.tls.insecure);
        let server_config =
            GrpcServerConfig::with_endpoint(&config.endpoint).with_tls_settings(tls_config);

        self.service_ref
            .get_service()
            .run_server(&server_config)
            .await
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to run server: {}", e),
            })
    }

    /// Connect to a SLIM server as a client (blocking version for FFI)
    ///
    /// # Arguments
    /// * `config` - Client configuration (endpoint and TLS settings)
    ///
    /// # Returns
    /// * `Ok(connection_id)` - Connected successfully, returns the connection ID
    /// * `Err(SlimError)` - If connection fails
    pub fn connect(&self, config: ClientConfig) -> Result<u64, SlimError> {
        self.runtime
            .block_on(async { self.connect_async(config).await })
    }

    /// Connect to a SLIM server (async version)
    ///
    /// Note: Automatically subscribes to the app's own name after connecting
    /// to enable receiving inbound messages and sessions.
    pub async fn connect_async(&self, config: ClientConfig) -> Result<u64, SlimError> {
        use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
        use slim_config::tls::client::TlsClientConfig;

        let tls_config = TlsClientConfig::new().with_insecure(config.tls.insecure);
        let client_config =
            GrpcClientConfig::with_endpoint(&config.endpoint).with_tls_setting(tls_config);

        let conn_id = self
            .service_ref
            .get_service()
            .connect(&client_config)
            .await
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to connect: {}", e),
            })?;

        // Automatically subscribe to our own name so we can receive messages.
        // If subscription fails, clean up the connection to avoid resource leaks.
        let self_name = self.app.app_name();
        if let Err(e) = self.app.subscribe(self_name, Some(conn_id)).await {
            let _ = self.service_ref.get_service().disconnect(conn_id);
            return Err(e.into());
        }

        Ok(conn_id)
    }

    /// Disconnect from a SLIM server (blocking version for FFI)
    ///
    /// # Arguments
    /// * `connection_id` - The connection ID returned from `connect()`
    ///
    /// # Returns
    /// * `Ok(())` - Disconnected successfully
    /// * `Err(SlimError)` - If disconnection fails
    pub fn disconnect(&self, connection_id: u64) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.disconnect_async(connection_id).await })
    }

    /// Disconnect from a SLIM server (async version)
    pub async fn disconnect_async(&self, connection_id: u64) -> Result<(), SlimError> {
        self.service_ref
            .get_service()
            .disconnect(connection_id)
            .map_err(|e| SlimError::ConfigError {
                message: format!("Failed to disconnect: {}", e),
            })
    }
}

// ============================================================================
// FFI SessionContext Wrapper
// ============================================================================

/// FFISessionContext represents an active session (FFI-compatible wrapper)
#[derive(uniffi::Object)]
pub struct FFISessionContext {
    /// The inner BindingsSessionContext (public for Python bindings access)
    pub inner: crate::bindings::BindingsSessionContext,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[uniffi::export]
impl FFISessionContext {
    /// Publish a message to the session (blocking version for FFI)
    pub fn publish(
        &self,
        destination: Name,
        fanout: u32,
        data: Vec<u8>,
        connection_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        self.runtime.block_on(async {
            self.publish_async(
                destination,
                fanout,
                data,
                connection_out,
                payload_type,
                metadata,
            )
            .await
        })
    }

    /// Publish a message to the session (async version)
    pub async fn publish_async(
        &self,
        destination: Name,
        fanout: u32,
        data: Vec<u8>,
        connection_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        let slim_dest: SlimName = destination.into();

        self.inner
            .publish(
                &slim_dest,
                fanout,
                data,
                connection_out,
                payload_type,
                metadata,
            )
            .await
            .map(|_| ())
            .map_err(|e| SlimError::SendError {
                message: e.to_string(),
            })
    }

    /// Receive a message from the session (blocking version for FFI)
    ///
    /// # Arguments
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Returns
    /// * `Ok(ReceivedMessage)` - Message with context and payload bytes
    /// * `Err(SlimError)` - If the receive fails or times out
    pub fn get_message(&self, timeout_ms: Option<u32>) -> Result<ReceivedMessage, SlimError> {
        self.runtime
            .block_on(async { self.get_message_async(timeout_ms).await })
    }

    /// Receive a message from the session (async version)
    pub async fn get_message_async(
        &self,
        timeout_ms: Option<u32>,
    ) -> Result<ReceivedMessage, SlimError> {
        let timeout = timeout_ms.map(|ms| std::time::Duration::from_millis(ms as u64));

        let (ctx, payload) =
            self.inner
                .get_session_message(timeout)
                .await
                .map_err(|e| SlimError::ReceiveError {
                    message: e.to_string(),
                })?;

        Ok(ReceivedMessage {
            context: MessageContext::from(ctx),
            payload,
        })
    }

    /// Invite a participant to the session (blocking version for FFI)
    pub fn invite(&self, participant: Name) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.invite_async(participant).await })
    }

    /// Invite a participant to the session (async version)
    pub async fn invite_async(&self, participant: Name) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.into();

        self.inner
            .invite(&slim_name)
            .await
            .map(|_| ())
            .map_err(|e| SlimError::SessionError {
                message: e.to_string(),
            })
    }

    /// Remove a participant from the session (blocking version for FFI)
    pub fn remove(&self, participant: Name) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.remove_async(participant).await })
    }

    /// Remove a participant from the session (async version)
    pub async fn remove_async(&self, participant: Name) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.into();

        self.inner
            .remove(&slim_name)
            .await
            .map(|_| ())
            .map_err(|e| SlimError::SessionError {
                message: e.to_string(),
            })
    }
}

// ============================================================================
// Internal methods for PyO3 bindings (not exported through UniFFI)
// ============================================================================

impl BindingsAdapter {
    /// Create a new session returning internal SessionContext (for PyO3 bindings)
    ///
    /// This method is for Python bindings to bypass the FFI layer and get
    /// a proper SessionContext with its CompletionHandle.
    pub async fn create_session_internal(
        &self,
        config: SlimSessionConfig,
        destination: SlimName,
    ) -> Result<
        (
            slim_session::context::SessionContext,
            slim_session::CompletionHandle,
        ),
        SlimError,
    > {
        let (session_ctx, completion) = self.app.create_session(config, destination, None).await?;

        Ok((session_ctx, completion))
    }

    /// Listen for incoming sessions returning internal SessionContext (for PyO3 bindings)
    pub async fn listen_for_session_internal(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<slim_session::context::SessionContext, SlimError> {
        let mut rx = self.notification_rx.write().await;

        let recv_fut = rx.recv();
        let notification_opt = if let Some(dur) = timeout {
            match tokio::time::timeout(dur, recv_fut).await {
                Ok(n) => n,
                Err(_) => {
                    return Err(SlimError::ReceiveError {
                        message: "listen_for_session timed out".to_string(),
                    });
                }
            }
        } else {
            recv_fut.await
        };

        if notification_opt.is_none() {
            return Err(SlimError::ReceiveError {
                message: "application channel closed".to_string(),
            });
        }

        match notification_opt.unwrap() {
            Ok(Notification::NewSession(ctx)) => Ok(ctx),
            Ok(Notification::NewMessage(_)) => Err(SlimError::ReceiveError {
                message: "received unexpected message notification while listening for session"
                    .to_string(),
            }),
            Err(e) => Err(SlimError::ReceiveError {
                message: format!("failed to receive session notification: {}", e),
            }),
        }
    }

    /// Delete a session returning CompletionHandle (for PyO3 bindings)
    ///
    /// This method is for Python bindings to get the CompletionHandle when deleting a session.
    pub fn delete_session_internal(
        &self,
        session: &SessionController,
    ) -> Result<slim_session::CompletionHandle, SlimError> {
        self.app
            .delete_session(session)
            .map_err(|e| SlimError::SessionError {
                message: format!("Failed to delete session: {}", e),
            })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::messages::Name as SlimName;
    use slim_testing::utils::TEST_VALID_SECRET;

    /// Test basic adapter creation
    #[tokio::test]
    async fn test_adapter_creation() {
        let base_name = SlimName::from_strings(["org", "namespace", "test-app"]);
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let result = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result.is_ok());

        let adapter = result.unwrap();
        assert!(adapter.id() > 0);
    }

    /// Test token ID generation
    #[tokio::test]
    async fn test_deterministic_id_generation() {
        let base_name = SlimName::from_strings(["org", "namespace", "test-app"]);
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let token_id = provider.get_id().expect("Failed to get token ID");
        let expected_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };

        let result = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result.is_ok());

        let adapter = result.unwrap();
        assert_eq!(adapter.id(), expected_hash);
    }
}
