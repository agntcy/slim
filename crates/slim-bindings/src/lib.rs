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

#[cfg(all(feature = "native", feature = "web"))]
compile_error!("features `native` and `web` are mutually exclusive");

// Single entry point for both native and browser (wasm32) builds. The divergent
// pieces (Service-backed vs WebSocket-backed bootstrap, blocking vs async-only
// FFI) are feature-gated inside the module rather than forked into a second file.
mod app;
mod build_info;
#[cfg(not(feature = "web"))]
mod client_config;
#[cfg(not(feature = "web"))]
mod common_config;
mod completion_handle;
#[cfg(not(feature = "web"))]
mod config;
mod errors;
#[cfg(not(feature = "web"))]
mod identity_config;
#[cfg(not(feature = "web"))]
mod init_config;
mod message_context;
mod name;
#[cfg(not(feature = "web"))]
mod server_config;
#[cfg(not(feature = "web"))]
mod service;
mod session;
#[cfg(not(feature = "web"))]
mod transport_protocol;

// Public re-exports
pub use app::{App, Direction, SessionWithCompletion};
pub use build_info::{BuildInfo, get_build_info, get_version};
#[cfg(not(feature = "web"))]
pub use client_config::{
    BackoffConfig, ClientConfig, ExponentialBackoff, KeepaliveConfig, ProxyConfig,
    new_config_from_json, new_insecure_client_config,
};
#[cfg(not(feature = "web"))]
pub use common_config::{
    BasicAuth, CaSource, ClientAuthenticationConfig, ServerAuthenticationConfig, SpireConfig,
    TlsClientConfig, TlsServerConfig, TlsSource,
};
pub use completion_handle::CompletionHandle;
#[cfg(not(feature = "web"))]
pub use config::get_runtime;
#[cfg(not(feature = "web"))]
pub use config::{
    get_global_service, get_runtime_config, get_service_config, get_tracing_config,
    initialize_from_config, initialize_with_configs, initialize_with_defaults, is_initialized,
    shutdown, shutdown_blocking,
};
pub use errors::SlimError;
#[cfg(not(feature = "web"))]
pub use identity_config::{
    ClientJwtAuth, IdentityProviderConfig, IdentityVerifierConfig, JwtAlgorithm, JwtAuth,
    JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType, StaticJwtAuth,
};
#[cfg(not(feature = "web"))]
pub use init_config::{
    RuntimeConfig, TracingConfig, new_runtime_config, new_runtime_config_with, new_service_config,
    new_service_config_with, new_tracing_config, new_tracing_config_with,
};
pub use message_context::{MessageContext, ReceivedMessage};
pub use name::Name;
#[cfg(not(feature = "web"))]
pub use server_config::{
    KeepaliveServerParameters, ServerConfig, new_insecure_server_config, new_server_config,
};
#[cfg(not(feature = "web"))]
pub use service::{
    DataplaneConfig, Service, ServiceConfig, create_service, create_service_with_config,
    new_dataplane_config, new_service_configuration,
};
pub use session::{MlsSettings, Session, SessionConfig, SessionType};
#[cfg(not(feature = "web"))]
pub use transport_protocol::TransportProtocol;
#[cfg(not(feature = "web"))]
pub use transport_protocol::TransportProtocol as ClientTransportProtocol;
#[cfg(not(feature = "web"))]
pub use transport_protocol::TransportProtocol as ServerTransportProtocol;

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
