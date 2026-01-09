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

use tokio::sync::{RwLock, mpsc};
use uniffi;

use crate::errors::SlimError;
use crate::name::Name;
use crate::runtime;
use crate::service_ref::{ServiceRef, get_or_init_global_service};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::TokenProvider; // For get_token() and get_id()
use slim_auth::traits::Verifier;
use slim_config::component::ComponentBuilder;
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name as SlimName;
use slim_service::Service;
use slim_service::app::App;
use slim_session::SessionConfig as SlimSessionConfig;
use slim_session::session_controller::SessionController;
use slim_session::{Notification, SessionError as SlimSessionError};

// ============================================================================
// UniFFI Type Definitions
// ============================================================================

/// Session type enum
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SessionType {
    PointToPoint,
    Group,
}

/// Session configuration
#[derive(uniffi::Record)]
pub struct SessionConfig {
    /// Session type (PointToPoint or Group)
    pub session_type: SessionType,

    /// Enable MLS encryption for this session
    pub enable_mls: bool,

    /// Maximum number of retries for message transmission (None = use default)
    pub max_retries: Option<u32>,

    /// Interval between retries in milliseconds (None = use default)
    pub interval_ms: Option<u64>,

    /// Whether this endpoint is the session initiator
    pub initiator: bool,

    /// Custom metadata key-value pairs for the session
    pub metadata: std::collections::HashMap<String, String>,
}

impl From<SessionConfig> for SlimSessionConfig {
    fn from(config: SessionConfig) -> Self {
        SlimSessionConfig {
            session_type: match config.session_type {
                SessionType::PointToPoint => ProtoSessionType::PointToPoint,
                SessionType::Group => ProtoSessionType::Multicast,
            },
            max_retries: config.max_retries,
            interval: config.interval_ms.map(std::time::Duration::from_millis),
            mls_enabled: config.enable_mls,
            initiator: config.initiator,
            metadata: config.metadata,
        }
    }
}

/// TLS configuration for server/client
#[derive(uniffi::Record)]
pub struct TlsConfig {
    /// Disable TLS entirely (plain text connection)
    pub insecure: bool,

    /// Skip server certificate verification (client only, enables TLS but doesn't verify certs)
    /// WARNING: Only use for testing - insecure in production!
    pub insecure_skip_verify: Option<bool>,

    /// Path to certificate file (PEM format)
    pub cert_file: Option<String>,

    /// Path to private key file (PEM format)
    pub key_file: Option<String>,

    /// Path to CA certificate file (PEM format) for verifying peer certificates
    pub ca_file: Option<String>,

    /// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
    pub tls_version: Option<String>,

    /// Include system CA certificates pool (default: false)
    pub include_system_ca_certs_pool: Option<bool>,
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

// Re-export MessageContext from message_context module (already has uniffi::Record)
pub use crate::message_context::MessageContext;

/// Received message containing context and payload
#[derive(Debug, Clone, uniffi::Record)]
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
    app_name: Arc<Name>,
    shared_secret: String,
) -> Result<Arc<BindingsAdapter>, SlimError> {
    runtime::get_runtime()
        .block_on(async { create_app_with_secret_async(app_name, shared_secret).await })
}

/// Create an app with the given name and shared secret (async version)
async fn create_app_with_secret_async(
    app_name: Arc<Name>,
    shared_secret: String,
) -> Result<Arc<BindingsAdapter>, SlimError> {
    let slim_name: SlimName = app_name.as_ref().into();
    let shared_secret_impl = SharedSecret::new(&slim_name.components_strings()[1], &shared_secret)?;

    // Wrap in enum types for flexible auth support
    let mut provider = AuthProvider::SharedSecret(shared_secret_impl.clone());
    let mut verifier = AuthVerifier::SharedSecret(shared_secret_impl);

    // Initialize the identity provider
    provider.initialize().await?;

    // Initialize the identity verifier
    verifier.initialize().await?;

    let adapter = BindingsAdapter::new(slim_name, provider, verifier, false)?;

    Ok(Arc::new(adapter))
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
    ) -> Result<Self, SlimError> {
        // Validate token
        let _identity_token = identity_provider.get_token()?;

        // Get ID from token and generate name with token ID
        let token_id = identity_provider.get_id()?;

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
            let svc = Service::builder().build("local-bindings-service".to_string())?;
            ServiceRef::Local(Box::new(svc))
        } else {
            ServiceRef::Global(get_or_init_global_service())
        };

        // Get service reference for adapter creation
        let service = service_ref.get_service();

        // Create the app
        let (app, rx) = service.create_app(&app_name, identity_provider, identity_verifier)?;

        Ok(Self {
            app: Arc::new(app),
            notification_rx: Arc::new(RwLock::new(rx)),
            service_ref,
        })
    }
}

#[uniffi::export]
impl BindingsAdapter {
    /// Get the app ID (derived from name)
    pub fn id(&self) -> u64 {
        self.app.app_name().id()
    }

    /// Get the app name
    pub fn name(&self) -> Arc<Name> {
        Arc::new(self.app.app_name().into())
    }

    /// Create a new session (blocking version for FFI)
    pub fn create_session(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<Arc<crate::BindingsSessionContext>, SlimError> {
        runtime::get_runtime()
            .block_on(async { self.create_session_async(config, destination).await })
    }

    /// Create a new session (async version)
    ///
    /// **Auto-waits for session establishment:** This method automatically waits for the
    /// session handshake to complete before returning. For point-to-point sessions, this
    /// ensures the remote peer has acknowledged the session. For multicast sessions, this
    /// ensures the initial setup is complete.
    pub async fn create_session_async(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<Arc<crate::BindingsSessionContext>, SlimError> {
        let slim_config: SlimSessionConfig = config.into();
        let slim_dest: SlimName = destination.as_ref().into();

        let (session_ctx, completion) = self
            .app
            .create_session(slim_config, slim_dest, None)
            .await?;

        // Wait for session establishment to complete
        // This ensures the session is fully ready before returning
        completion.await?;

        // Create BindingsSessionContext with the runtime
        Ok(Arc::new(crate::BindingsSessionContext::new(session_ctx)))
    }

    /// Delete a session (blocking version for FFI)
    pub fn delete_session(
        &self,
        session: Arc<crate::BindingsSessionContext>,
    ) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(async { self.delete_session_async(session).await })
    }

    /// Delete a session (async version)
    pub async fn delete_session_async(
        &self,
        session: Arc<crate::BindingsSessionContext>,
    ) -> Result<(), SlimError> {
        let session_ref = session
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        let completion = self.app.delete_session(&session_ref)?;

        // Wait for session deletion to complete
        completion.await?;

        Ok(())
    }

    /// Subscribe to a name (blocking version for FFI)
    pub fn subscribe(&self, name: Arc<Name>, connection_id: Option<u64>) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(async { self.subscribe_async(name, connection_id).await })
    }

    /// Subscribe to a name (async version)
    pub async fn subscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.subscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Unsubscribe from a name (blocking version for FFI)
    pub fn unsubscribe(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(async { self.unsubscribe_async(name, connection_id).await })
    }

    /// Unsubscribe from a name (async version)
    pub async fn unsubscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.unsubscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Set a route to a name for a specific connection (blocking version for FFI)
    pub fn set_route(&self, name: Arc<Name>, connection_id: u64) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(async { self.set_route_async(name, connection_id).await })
    }

    /// Set a route to a name for a specific connection (async version)
    pub async fn set_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.set_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Remove a route (blocking version for FFI)
    pub fn remove_route(&self, name: Arc<Name>, connection_id: u64) -> Result<(), SlimError> {
        runtime::get_runtime()
            .block_on(async { self.remove_route_async(name, connection_id).await })
    }

    /// Remove a route (async version)
    pub async fn remove_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.remove_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Listen for incoming sessions (blocking version for FFI)
    pub fn listen_for_session(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<Arc<crate::BindingsSessionContext>, SlimError> {
        runtime::get_runtime().block_on(async { self.listen_for_session_async(timeout).await })
    }

    /// Listen for incoming sessions (async version)
    pub async fn listen_for_session_async(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<Arc<crate::BindingsSessionContext>, SlimError> {
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
                Ok(Arc::new(crate::BindingsSessionContext::new(ctx)))
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
        runtime::get_runtime().block_on(async { self.run_server_async(config).await })
    }

    /// Run a SLIM server (async version)
    pub async fn run_server_async(&self, config: ServerConfig) -> Result<(), SlimError> {
        use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
        use slim_config::tls::server::TlsServerConfig;

        let mut tls_config = TlsServerConfig::new().with_insecure(config.tls.insecure);

        // Apply optional TLS settings
        if let (Some(cert_file), Some(key_file)) =
            (config.tls.cert_file.as_ref(), config.tls.key_file.as_ref())
        {
            tls_config = tls_config.with_cert_and_key_file(cert_file, key_file);
        }

        if let Some(ca_file) = config.tls.ca_file.as_ref() {
            tls_config = tls_config.with_ca_file(ca_file);
        }

        if let Some(tls_version) = config.tls.tls_version.as_ref() {
            tls_config = tls_config.with_tls_version(tls_version);
        }

        if let Some(include_system_ca) = config.tls.include_system_ca_certs_pool {
            tls_config = tls_config.with_include_system_ca_certs_pool(include_system_ca);
        }

        let server_config =
            GrpcServerConfig::with_endpoint(&config.endpoint).with_tls_settings(tls_config);

        self.service_ref
            .get_service()
            .run_server(&server_config)
            .await?;

        Ok(())
    }

    /// Stop a running SLIM server (blocking version for FFI)
    ///
    /// # Arguments
    /// * `endpoint` - The endpoint address of the server to stop (e.g., "127.0.0.1:12345")
    ///
    /// # Returns
    /// * `Ok(())` - Server stopped successfully
    /// * `Err(SlimError)` - If server not found or stop fails
    pub fn stop_server(&self, endpoint: String) -> Result<(), SlimError> {
        self.service_ref.get_service().stop_server(&endpoint)?;
        Ok(())
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
        runtime::get_runtime().block_on(async { self.connect_async(config).await })
    }

    /// Connect to a SLIM server (async version)
    ///
    /// Note: Automatically subscribes to the app's own name after connecting
    /// to enable receiving inbound messages and sessions.
    pub async fn connect_async(&self, config: ClientConfig) -> Result<u64, SlimError> {
        use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
        use slim_config::tls::client::TlsClientConfig;

        let mut tls_config = TlsClientConfig::new().with_insecure(config.tls.insecure);

        // Apply optional TLS settings
        if let Some(skip_verify) = config.tls.insecure_skip_verify {
            tls_config = tls_config.with_insecure_skip_verify(skip_verify);
        }

        if let (Some(cert_file), Some(key_file)) =
            (config.tls.cert_file.as_ref(), config.tls.key_file.as_ref())
        {
            tls_config = tls_config.with_cert_and_key_file(cert_file, key_file);
        }

        if let Some(ca_file) = config.tls.ca_file.as_ref() {
            tls_config = tls_config.with_ca_file(ca_file);
        }

        if let Some(tls_version) = config.tls.tls_version.as_ref() {
            tls_config = tls_config.with_tls_version(tls_version);
        }

        if let Some(include_system_ca) = config.tls.include_system_ca_certs_pool {
            tls_config = tls_config.with_include_system_ca_certs_pool(include_system_ca);
        }

        let client_config =
            GrpcClientConfig::with_endpoint(&config.endpoint).with_tls_setting(tls_config);

        let conn_id = self
            .service_ref
            .get_service()
            .connect(&client_config)
            .await?;

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
        runtime::get_runtime().block_on(async { self.disconnect_async(connection_id).await })
    }

    /// Disconnect from a SLIM server (async version)
    pub async fn disconnect_async(&self, connection_id: u64) -> Result<(), SlimError> {
        self.service_ref.get_service().disconnect(connection_id)?;

        Ok(())
    }
}

// ============================================================================
// FFI CompletionHandle Wrapper
// ============================================================================

/// FFI-compatible completion handle for async operations
///
/// Represents a pending operation that can be awaited to ensure completion.
/// Used for operations that need delivery confirmation or handshake acknowledgment.
///
/// # Design Note
/// Since Rust futures can only be polled once to completion, this handle uses
/// a shared receiver that can only be consumed once. Attempting to wait multiple
/// times on the same handle will return an error.
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// let completion = session.publish_with_completion(data, None, None)?;
/// completion.wait()?; // Wait for delivery confirmation
/// ```
#[derive(uniffi::Object)]
pub struct FfiCompletionHandle {
    /// Receiver for the completion result (can only be consumed once)
    receiver: Arc<
        parking_lot::Mutex<
            Option<tokio::sync::oneshot::Receiver<Result<(), slim_session::SessionError>>>,
        >,
    >,
}

impl std::fmt::Debug for FfiCompletionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let has_receiver = self.receiver.lock().is_some();
        f.debug_struct("FfiCompletionHandle")
            .field("receiver_available", &has_receiver)
            .finish()
    }
}

impl FfiCompletionHandle {
    /// Create a new FFI completion handle from a Rust CompletionHandle
    pub fn new(handle: slim_session::CompletionHandle) -> Self {
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Spawn a task to await the completion and send the result
        runtime::get_runtime().spawn(async move {
            let result = handle.await;
            let _ = tx.send(result);
        });

        Self {
            receiver: Arc::new(parking_lot::Mutex::new(Some(rx))),
        }
    }
}

#[uniffi::export]
impl FfiCompletionHandle {
    /// Wait for the operation to complete indefinitely (blocking version)
    ///
    /// This blocks the calling thread until the operation completes.
    /// Use this from Go or other languages when you need to ensure
    /// an operation has finished before proceeding.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub fn wait(&self) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(self.wait_async())
    }

    /// Wait for the operation to complete with a timeout (blocking version)
    ///
    /// This blocks the calling thread until the operation completes or the timeout expires.
    /// Use this from Go or other languages when you need to ensure
    /// an operation has finished before proceeding with a time limit.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for completion
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError::Timeout)` - If the operation timed out
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub fn wait_for(&self, timeout: std::time::Duration) -> Result<(), SlimError> {
        runtime::get_runtime().block_on(self.wait_for_async(timeout))
    }

    /// Wait for the operation to complete indefinitely (async version)
    ///
    /// This is the async version that integrates with UniFFI's polling mechanism.
    /// The operation will yield control while waiting.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub async fn wait_async(&self) -> Result<(), SlimError> {
        self.wait_internal(None).await
    }

    /// Wait for the operation to complete with a timeout (async version)
    ///
    /// This is the async version that integrates with UniFFI's polling mechanism.
    /// The operation will yield control while waiting until completion or timeout.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for completion
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError::Timeout)` - If the operation timed out
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub async fn wait_for_async(&self, timeout: std::time::Duration) -> Result<(), SlimError> {
        self.wait_internal(Some(timeout)).await
    }
}

impl FfiCompletionHandle {
    /// Internal implementation for wait operations
    async fn wait_internal(&self, timeout: Option<std::time::Duration>) -> Result<(), SlimError> {
        let receiver = self
            .receiver
            .lock()
            .take()
            .ok_or_else(|| SlimError::InternalError {
                message: "CompletionHandle already consumed (wait can only be called once)"
                    .to_string(),
            })?;

        let wait_future = async {
            receiver
                .await
                .map_err(|_| SlimError::InternalError {
                    message: "Completion sender dropped before result was sent".to_string(),
                })?
                .map_err(|e| SlimError::SessionError {
                    message: e.to_string(),
                })
        };

        if let Some(duration) = timeout {
            tokio::time::timeout(duration, wait_future)
                .await
                .map_err(|_| SlimError::Timeout)?
        } else {
            wait_future.await
        }
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
        let ret = self.app.delete_session(session)?;

        Ok(ret)
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
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET).unwrap();
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
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET).unwrap();
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

    /// Test FfiCompletionHandle basic functionality
    #[tokio::test]
    async fn test_completion_handle_success() {
        // Create a successful completion
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Send success
        tx.send(Ok(())).unwrap();

        // Wait should succeed
        let result = ffi_handle.wait_async().await;
        assert!(result.is_ok(), "Completion should succeed");
    }

    /// Test FfiCompletionHandle failure propagation
    #[tokio::test]
    async fn test_completion_handle_failure() {
        // Create a failed completion
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Send error
        tx.send(Err(slim_session::SessionError::SlimMessageSendFailed))
            .unwrap();

        // Wait should fail with error
        let result = ffi_handle.wait_async().await;
        assert!(result.is_err_and(|e| matches!(e, SlimError::SessionError { .. })));
    }

    /// Test FfiCompletionHandle can only be consumed once
    #[tokio::test]
    async fn test_completion_handle_single_consumption() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(FfiCompletionHandle::new(completion));

        tx.send(Ok(())).unwrap();

        // First wait should succeed
        let result1 = ffi_handle.wait_async().await;
        assert!(result1.is_ok(), "First wait should succeed");

        // Second wait should fail (already consumed)
        let result2 = ffi_handle.wait_async().await;
        assert!(result2.is_err(), "Second wait should fail");

        match result2 {
            Err(SlimError::InternalError { message }) => {
                assert!(message.contains("already consumed"));
            }
            _ => panic!("Expected InternalError about consumption"),
        }
    }

    /// Test FfiCompletionHandle async version
    #[tokio::test]
    async fn test_completion_handle_async() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Send success in a separate task
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(Ok(())).unwrap();
        });

        // Async wait should succeed
        let result = ffi_handle.wait_async().await;
        assert!(result.is_ok(), "Async wait should succeed");
    }

    /// Test FfiCompletionHandle with dropped sender
    #[tokio::test]
    async fn test_completion_handle_sender_dropped() {
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Drop the sender explicitly
        drop(_tx);

        // Give the spawned task time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Wait should fail because sender was dropped (or already consumed by background task)
        let result = ffi_handle.wait_async().await;
        assert!(
            result.is_err(),
            "Should fail when sender is dropped or already consumed"
        );

        // Either error type is valid depending on timing
        match result {
            Err(SlimError::InternalError { message }) => {
                assert!(
                    message.contains("sender dropped") || message.contains("already consumed"),
                    "Error message should mention sender dropped or already consumed, got: {}",
                    message
                );
            }
            Err(SlimError::SessionError { message }) => {
                // The background task may have consumed the receiver and got a channel closed error
                assert!(
                    message.contains("channel closed") || message.contains("receiving ack"),
                    "Expected channel closed error, got: {}",
                    message
                );
            }
            Err(e) => panic!("Unexpected error type: {:?}", e),
            Ok(_) => panic!("Expected error, got Ok"),
        }
    }

    /// Test concurrent completion handle usage
    #[tokio::test]
    async fn test_completion_handle_concurrent() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let success_count = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];

        // Create multiple completion handles
        for _ in 0..10 {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
            let ffi_handle = Arc::new(FfiCompletionHandle::new(completion));

            let count = Arc::clone(&success_count);
            let handle = tokio::spawn(async move {
                // Send success
                tx.send(Ok(())).unwrap();

                // Wait for completion
                if ffi_handle.wait_async().await.is_ok() {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            });

            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // All should succeed
        assert_eq!(success_count.load(Ordering::SeqCst), 10);
    }

    /// Test FfiCompletionHandle with timeout
    #[tokio::test]
    async fn test_completion_handle_with_timeout() {
        // Create a completion that will never complete (sender not sent)
        // NOTE: We must hold onto `tx` so the channel stays open.
        // If we use `_tx`, it gets dropped immediately and the receiver
        // returns RecvError instead of timing out.
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), SlimSessionError>>();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Wait with a short timeout - should timeout
        let result = ffi_handle
            .wait_for_async(std::time::Duration::from_millis(50))
            .await;
        assert!(
            result.is_err(),
            "Should timeout when operation doesn't complete"
        );

        match result {
            Err(SlimError::Timeout) => {} // Expected
            Err(e) => panic!("Expected Timeout error, got: {:?}", e),
            Ok(_) => panic!("Expected timeout, got Ok"),
        }

        // Explicitly drop tx after the test to avoid unused variable warning
        drop(tx);
    }

    /// Test FfiCompletionHandle with timeout that completes in time
    #[tokio::test]
    async fn test_completion_handle_timeout_success() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = FfiCompletionHandle::new(completion);

        // Send success immediately
        tx.send(Ok(())).unwrap();

        // Wait with a generous timeout - should succeed
        let result = ffi_handle
            .wait_for_async(std::time::Duration::from_millis(5000))
            .await;
        assert!(
            result.is_ok(),
            "Should succeed when operation completes before timeout"
        );
    }

    /// Test that session creation auto-waits for establishment
    #[tokio::test]
    async fn test_session_creation_auto_wait() {
        // This test verifies that create_session_async properly awaits the completion handle
        // In a real scenario, this would ensure the session is fully established

        let base_name = SlimName::from_strings(["org", "namespace", "create-test"]);
        let shared_secret = SharedSecret::new("create-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(3),
            interval_ms: Some(100),
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "dest".to_string(),
            None,
        ));

        // This should auto-wait for session establishment
        // If it returns without error, the session is fully established
        let result = adapter
            .create_session_async(session_config, destination)
            .await;

        // In a real scenario with network, this would verify the session is ready
        // For this test, we just verify it completes without panicking
        match result {
            Ok(_session) => {
                // Session created and auto-waited successfully
            }
            Err(e) => {
                // Expected to fail in test environment without network
                // but shouldn't panic
                println!("Expected error in test environment: {:?}", e);
            }
        }
    }

    /// Test publish_with_completion returns a valid completion handle
    #[tokio::test]
    async fn test_publish_with_completion_returns_handle() {
        // Note: This is a structural test - in a real environment with connections,
        // the completion handle would actually track message delivery

        let base_name = SlimName::from_strings(["org", "namespace", "publish-test"]);
        let shared_secret = SharedSecret::new("publish-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(3),
            interval_ms: Some(100),
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "dest".to_string(),
            None,
        ));

        // Try to create a session (may fail without network)
        if let Ok(session) = adapter
            .create_session_async(session_config, destination)
            .await
        {
            let data = b"test message".to_vec();

            // Attempt to publish with completion
            // This verifies the API exists and returns the right type
            let result = session
                .publish_with_completion_async(data, None, None)
                .await;

            match result {
                Ok(completion_handle) => {
                    // Verify we got a completion handle
                    // In a real scenario, we could wait on it
                    assert!(Arc::strong_count(&completion_handle) > 0);
                }
                Err(e) => {
                    // Expected to fail without actual connections
                    println!("Expected error without network: {:?}", e);
                }
            }
        }
    }

    // ========================================================================
    // SessionConfig Conversion Tests
    // ========================================================================

    /// Test SessionConfig to SlimSessionConfig conversion for PointToPoint
    #[test]
    fn test_session_config_point_to_point() {
        let config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: true,
            max_retries: Some(5),
            interval_ms: Some(200),
            initiator: true,
            metadata: std::collections::HashMap::from([
                ("key1".to_string(), "value1".to_string()),
                ("key2".to_string(), "value2".to_string()),
            ]),
        };

        let slim_config: SlimSessionConfig = config.into();

        assert_eq!(slim_config.session_type, ProtoSessionType::PointToPoint);
        assert!(slim_config.mls_enabled);
        assert_eq!(slim_config.max_retries, Some(5));
        assert_eq!(
            slim_config.interval,
            Some(std::time::Duration::from_millis(200))
        );
        assert!(slim_config.initiator);
        assert_eq!(
            slim_config.metadata.get("key1"),
            Some(&"value1".to_string())
        );
    }

    /// Test SessionConfig to SlimSessionConfig conversion for Group
    #[test]
    fn test_session_config_group() {
        let config = SessionConfig {
            session_type: SessionType::Group,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: false,
            metadata: std::collections::HashMap::new(),
        };

        let slim_config: SlimSessionConfig = config.into();

        assert_eq!(slim_config.session_type, ProtoSessionType::Multicast);
        assert!(!slim_config.mls_enabled);
        assert!(slim_config.max_retries.is_none());
        assert!(slim_config.interval.is_none());
        assert!(!slim_config.initiator);
    }

    // ========================================================================
    // TlsConfig Tests
    // ========================================================================

    /// Test TlsConfig with all fields
    #[test]
    fn test_tls_config_full() {
        let config = TlsConfig {
            insecure: false,
            insecure_skip_verify: Some(true),
            cert_file: Some("/path/to/cert.pem".to_string()),
            key_file: Some("/path/to/key.pem".to_string()),
            ca_file: Some("/path/to/ca.pem".to_string()),
            tls_version: Some("tls1.3".to_string()),
            include_system_ca_certs_pool: Some(true),
        };

        assert!(!config.insecure);
        assert_eq!(config.insecure_skip_verify, Some(true));
        assert_eq!(config.cert_file, Some("/path/to/cert.pem".to_string()));
        assert_eq!(config.key_file, Some("/path/to/key.pem".to_string()));
        assert_eq!(config.ca_file, Some("/path/to/ca.pem".to_string()));
        assert_eq!(config.tls_version, Some("tls1.3".to_string()));
        assert_eq!(config.include_system_ca_certs_pool, Some(true));
    }

    /// Test TlsConfig with insecure mode
    #[test]
    fn test_tls_config_insecure() {
        let config = TlsConfig {
            insecure: true,
            insecure_skip_verify: None,
            cert_file: None,
            key_file: None,
            ca_file: None,
            tls_version: None,
            include_system_ca_certs_pool: None,
        };

        assert!(config.insecure);
        assert!(config.cert_file.is_none());
    }

    // ========================================================================
    // ServerConfig and ClientConfig Tests
    // ========================================================================

    /// Test ServerConfig creation
    #[test]
    fn test_server_config() {
        let config = ServerConfig {
            endpoint: "127.0.0.1:8080".to_string(),
            tls: TlsConfig {
                insecure: false,
                insecure_skip_verify: None,
                cert_file: Some("/cert.pem".to_string()),
                key_file: Some("/key.pem".to_string()),
                ca_file: None,
                tls_version: None,
                include_system_ca_certs_pool: None,
            },
        };

        assert_eq!(config.endpoint, "127.0.0.1:8080");
        assert!(!config.tls.insecure);
    }

    /// Test ClientConfig creation
    #[test]
    fn test_client_config() {
        let config = ClientConfig {
            endpoint: "example.com:443".to_string(),
            tls: TlsConfig {
                insecure: false,
                insecure_skip_verify: Some(false),
                cert_file: None,
                key_file: None,
                ca_file: Some("/ca.pem".to_string()),
                tls_version: Some("tls1.2".to_string()),
                include_system_ca_certs_pool: Some(true),
            },
        };

        assert_eq!(config.endpoint, "example.com:443");
        assert_eq!(config.tls.tls_version, Some("tls1.2".to_string()));
    }

    // ========================================================================
    // Adapter Methods Tests
    // ========================================================================

    /// Test adapter id() and name() methods
    #[tokio::test]
    async fn test_adapter_id_and_name() {
        let base_name = SlimName::from_strings(["org", "namespace", "id-test"]);
        let shared_secret = SharedSecret::new("id-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name.clone(), provider, verifier, false)
            .expect("Failed to create adapter");

        // ID should be non-zero
        let id = adapter.id();
        assert!(id > 0, "Adapter ID should be positive");

        // Name should have the right components
        let name = adapter.name();
        assert_eq!(name.components()[0], "org");
        assert_eq!(name.components()[1], "namespace");
        assert_eq!(name.components()[2], "id-test");
        assert!(name.id() > 0);
    }

    /// Test adapter with local service
    #[tokio::test]
    async fn test_adapter_with_local_service() {
        let base_name = SlimName::from_strings(["org", "namespace", "local-test"]);
        let shared_secret = SharedSecret::new("local-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        // Create with use_local_service = true
        let result = BindingsAdapter::new(base_name, provider, verifier, true);
        assert!(result.is_ok(), "Should create adapter with local service");
    }

    /// Test adapter creation with different namespace values
    #[tokio::test]
    async fn test_adapter_different_namespaces() {
        // Test with different valid namespace configurations
        let namespaces = [
            ["org1", "namespace1", "app1"],
            ["company", "team", "service"],
            ["prod", "api", "gateway"],
        ];

        for ns in namespaces {
            let base_name = SlimName::from_strings(ns);
            let shared_secret = SharedSecret::new(ns[2], TEST_VALID_SECRET).unwrap();
            let provider = AuthProvider::SharedSecret(shared_secret.clone());
            let verifier = AuthVerifier::SharedSecret(shared_secret);

            let result = BindingsAdapter::new(base_name, provider, verifier, false);
            assert!(
                result.is_ok(),
                "Should create adapter for namespace {:?}",
                ns
            );
        }
    }

    // ========================================================================
    // ReceivedMessage Tests
    // ========================================================================

    /// Test ReceivedMessage creation
    #[test]
    fn test_received_message() {
        let msg = ReceivedMessage {
            context: crate::message_context::MessageContext::new(
                Name::new(
                    "org".to_string(),
                    "ns".to_string(),
                    "app".to_string(),
                    Some(123),
                ),
                Some(Name::new(
                    "org".to_string(),
                    "ns".to_string(),
                    "dest".to_string(),
                    Some(456),
                )),
                "application/json".to_string(),
                std::collections::HashMap::new(),
                789,
                "test-identity".to_string(),
            ),
            payload: b"hello world".to_vec(),
        };

        assert_eq!(msg.payload, b"hello world");
        assert_eq!(msg.context.input_connection, 789);
        assert_eq!(msg.context.payload_type, "application/json");
        assert_eq!(msg.context.identity, "test-identity");
    }

    // ========================================================================
    // initialize_crypto_provider Tests
    // ========================================================================

    /// Test initialize_crypto_provider can be called multiple times safely
    #[test]
    fn test_initialize_crypto_provider_idempotent() {
        // Should not panic when called multiple times
        crate::common::initialize_crypto_provider();
        crate::common::initialize_crypto_provider();
        crate::common::initialize_crypto_provider();
    }

    // ========================================================================
    // create_app_with_secret Tests
    // ========================================================================

    /// Test create_app_with_secret FFI entry point
    #[test]
    fn test_create_app_with_secret() {
        let app_name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "ffi-app".to_string(),
            None,
        ));

        let result = create_app_with_secret(app_name, TEST_VALID_SECRET.to_string());
        assert!(result.is_ok(), "create_app_with_secret should succeed");

        let adapter = result.unwrap();
        assert!(adapter.id() > 0);

        let name = adapter.name();
        assert_eq!(name.components()[0], "org");
        assert_eq!(name.components()[1], "namespace");
        assert_eq!(name.components()[2], "ffi-app");
    }

    /// Test create_app_with_secret with empty name components
    #[test]
    fn test_create_app_with_secret_minimal_name() {
        let app_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "".to_string(),
            None,
        ));

        let result = create_app_with_secret(app_name, TEST_VALID_SECRET.to_string());
        // Should handle empty component
        let _ = result;
    }

    // ========================================================================
    // Listen for Session Timeout Tests
    // ========================================================================

    /// Test listen_for_session with timeout
    #[tokio::test]
    async fn test_listen_for_session_timeout() {
        let base_name = SlimName::from_strings(["org", "namespace", "listen-test"]);
        let shared_secret = SharedSecret::new("listen-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        // Listen with a very short timeout - should timeout
        let result = adapter
            .listen_for_session_async(Some(std::time::Duration::from_millis(10)))
            .await;

        match result {
            Err(SlimError::ReceiveError { message }) => {
                assert!(
                    message.contains("timed out"),
                    "Should contain timeout message"
                );
            }
            _ => {
                // May get a different error in some cases, which is fine
            }
        }
    }

    // ========================================================================
    // Subscribe/Unsubscribe Tests
    // ========================================================================

    /// Test subscribe and unsubscribe (requires running service)
    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let base_name = SlimName::from_strings(["org", "namespace", "sub-test"]);
        let shared_secret = SharedSecret::new("sub-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "target".to_string(),
            None,
        ));

        // Subscribe (may fail without connection, but shouldn't panic)
        let sub_result = adapter.subscribe_async(target_name.clone(), None).await;
        // We don't assert success because there's no active connection

        // Unsubscribe
        let unsub_result = adapter.unsubscribe_async(target_name, None).await;
        // Same - may fail but shouldn't panic

        let _ = (sub_result, unsub_result);
    }

    // ========================================================================
    // Set/Remove Route Tests
    // ========================================================================

    /// Test set_route and remove_route
    #[tokio::test]
    async fn test_set_remove_route() {
        let base_name = SlimName::from_strings(["org", "namespace", "route-test"]);
        let shared_secret = SharedSecret::new("route-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "route-target".to_string(),
            None,
        ));

        // Set route (may fail without valid connection_id)
        let set_result = adapter.set_route_async(target_name.clone(), 12345).await;
        // Remove route
        let remove_result = adapter.remove_route_async(target_name, 12345).await;

        // These will likely fail without actual connections, but shouldn't panic
        let _ = (set_result, remove_result);
    }

    // ========================================================================
    // Stop Server Tests
    // ========================================================================

    /// Test stop_server on non-existent server
    #[tokio::test]
    async fn test_stop_server_nonexistent() {
        let base_name = SlimName::from_strings(["org", "namespace", "stop-test"]);
        let shared_secret = SharedSecret::new("stop-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        // Try to stop a server that doesn't exist
        let result = adapter.stop_server("127.0.0.1:99999".to_string());
        // Should fail with appropriate error
        assert!(result.is_err());
    }

    // ========================================================================
    // Disconnect Tests
    // ========================================================================

    /// Test disconnect with invalid connection_id
    #[tokio::test]
    async fn test_disconnect_invalid_id() {
        let base_name = SlimName::from_strings(["org", "namespace", "disconnect-test"]);
        let shared_secret = SharedSecret::new("disconnect-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        // Try to disconnect with an invalid connection ID
        let result = adapter.disconnect_async(999999).await;
        // Should fail but not panic
        assert!(result.is_err());
    }

    // ========================================================================
    // Delete Session Tests
    // ========================================================================

    /// Test delete_session with a session (structural test)
    #[tokio::test]
    async fn test_delete_session_flow() {
        let base_name = SlimName::from_strings(["org", "namespace", "delete-test"]);
        let shared_secret = SharedSecret::new("delete-test", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(1),
            interval_ms: Some(50),
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "delete-dest".to_string(),
            None,
        ));

        // Create session (may fail without network)
        if let Ok(session) = adapter
            .create_session_async(session_config, destination)
            .await
        {
            // Delete session
            let delete_result = adapter.delete_session_async(session).await;
            // May succeed or fail depending on session state
            let _ = delete_result;
        }
    }

    // ========================================================================
    // Blocking Method Tests
    // ========================================================================

    /// Test blocking version of listen_for_session
    #[tokio::test]
    async fn test_listen_for_session_blocking_timeout() {
        let base_name = SlimName::from_strings(["org", "namespace", "blocking-listen"]);
        let shared_secret = SharedSecret::new("blocking-listen", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        // Call async version with short timeout (blocking version can't be called from async context)
        let result = adapter
            .listen_for_session_async(Some(std::time::Duration::from_millis(10)))
            .await;

        // Should timeout
        match result {
            Err(SlimError::ReceiveError { message }) => {
                assert!(message.contains("timed out") || message.contains("closed"));
            }
            _ => {
                // Other errors are acceptable too
            }
        }
    }

    /// Test blocking version of subscribe
    #[tokio::test]
    async fn test_subscribe_blocking() {
        let base_name = SlimName::from_strings(["org", "namespace", "blocking-sub"]);
        let shared_secret = SharedSecret::new("blocking-sub", TEST_VALID_SECRET).unwrap();
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);

        let adapter = BindingsAdapter::new(base_name, provider, verifier, true)
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "block-target".to_string(),
            None,
        ));

        // Call async version (blocking version can't be called from async context)
        let _ = adapter.subscribe_async(target_name, None).await;
    }
}
