// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SLIM Bindings - UniFFI Language Bindings
//!
//! This crate provides language-agnostic FFI bindings for SLIM using UniFFI.
//! It enables integration with Go, Python, Kotlin, Swift, and other languages.
//!
//! ## Architecture
//!
//! The module is organized into distinct components:
//!
//! - **`App`**: App-level operations (creation, configuration, session management)
//! - **`Session`**: Session-specific operations (publish, invite, remove, message reception)
//! - **`MessageContext`**: Message metadata and routing information
//! - **`ServiceRef`**: Service reference management (global vs local)
//!
//! ## Usage
//!
//! ### From Rust
//!
//! ```rust,ignore
//! use slim_bindings::{App, Name, SessionConfig, SessionType, IdentityProviderConfig, IdentityVerifierConfig};
//!
//! // Create an app
//! let app_name = Arc::new(Name { components: vec!["org".into(), "app".into(), "v1".into()], id: None });
//! let provider_config = IdentityProviderConfig::SharedSecret { data: "my-secret".to_string() };
//! let verifier_config = IdentityVerifierConfig::SharedSecret { data: "my-secret".to_string() };
//! let app = App::new(app_name, provider_config, verifier_config, false)?;
//!
//! // Create a session
//! let config = SessionConfig { session_type: SessionType::PointToPoint, ... };
//! let session = app.create_session(config, destination)?;
//! ```
//!
//! ### From Go (via generated bindings)
//!
//! ```go
//! providerConfig := slim.IdentityProviderConfigSharedSecret{Data: sharedSecret}
//! verifierConfig := slim.IdentityVerifierConfigSharedSecret{Data: sharedSecret}
//! app, err := slim.NewApp(appName, providerConfig, verifierConfig, false)
//! session, err := app.CreateSession(config, destination)
//! session.Publish(data, payloadType, metadata)
//! ```

// Module declarations
mod app;
mod build_info;
mod client_config;
mod common_config;
mod completion_handle;
mod config;
mod errors;
mod identity_config;
mod init_config;
mod message_context;
mod name;
mod server_config;
mod service;
mod session;

// SlimRPC module (unified core + UniFFI bindings)
pub mod slimrpc;

// Public re-exports
pub use app::{App, Direction, SessionWithCompletion};
pub use build_info::{BuildInfo, get_build_info, get_version};
pub use client_config::{
    BackoffConfig, ClientConfig, ExponentialBackoff, KeepaliveConfig, ProxyConfig,
    new_insecure_client_config,
};
pub use common_config::{
    BasicAuth, CaSource, ClientAuthenticationConfig, ServerAuthenticationConfig, SpireConfig,
    TlsClientConfig, TlsServerConfig, TlsSource,
};
pub use completion_handle::CompletionHandle;
pub use config::get_runtime;
pub use config::{
    get_global_service, get_runtime_config, get_service_config, get_tracing_config,
    initialize_from_config, initialize_with_configs, initialize_with_defaults, is_initialized,
    shutdown, shutdown_blocking,
};
pub use errors::SlimError;
pub use identity_config::{
    ClientJwtAuth, IdentityProviderConfig, IdentityVerifierConfig, JwtAlgorithm, JwtAuth,
    JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType, StaticJwtAuth,
};
pub use init_config::{
    RuntimeConfig, TracingConfig, new_runtime_config, new_runtime_config_with, new_service_config,
    new_service_config_with, new_tracing_config, new_tracing_config_with,
};
pub use message_context::{MessageContext, ReceivedMessage};
pub use name::Name;
pub use server_config::{
    KeepaliveServerParameters, ServerConfig, new_insecure_server_config, new_server_config,
};
pub use service::{
    DataplaneConfig, Service, ServiceConfig, create_service, create_service_with_config,
    new_dataplane_config, new_service_configuration,
};
pub use session::{Session, SessionConfig, SessionType};

// SLIMRpc re-exports
pub use slimrpc::{
    BidiStreamHandler, Channel, Codec, Context, DEADLINE_KEY, Decoder, Encoder, HandlerResponse,
    HandlerType, InvalidRpcCode, MAX_TIMEOUT, RequestStreamWriter, ResponseSink, ResponseStream,
    ResponseStreamReader, RpcCode, RpcError, STATUS_CODE_KEY, Server,
    SessionContext as RpcSessionContext, StreamMessage, StreamStreamHandler, StreamUnaryHandler,
    UnaryStreamHandler, UnaryUnaryHandler, UniffiRequestStream as RequestStream,
    build_method_subscription_name,
};

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
