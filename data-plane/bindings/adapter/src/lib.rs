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
//! - **`BindingsAdapter`**: App-level operations (creation, configuration, session management)
//! - **`BindingsSessionContext`**: Session-specific operations (publish, invite, remove, message reception)
//! - **`MessageContext`**: Message metadata and routing information
//! - **`ServiceRef`**: Service reference management (global vs local)
//!
//! ## Usage
//!
//! ### From Rust
//!
//! ```rust,ignore
//! use slim_bindings::{BindingsAdapter, Name, SessionConfig, SessionType, IdentityProviderConfig, IdentityVerifierConfig};
//!
//! // Create an app
//! let app_name = Arc::new(Name { components: vec!["org".into(), "app".into(), "v1".into()], id: None });
//! let provider_config = IdentityProviderConfig::SharedSecret { data: "my-secret".to_string() };
//! let verifier_config = IdentityVerifierConfig::SharedSecret { data: "my-secret".to_string() };
//! let app = BindingsAdapter::new(app_name, provider_config, verifier_config, false)?;
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
//! app, err := slim.NewBindingsAdapter(appName, providerConfig, verifierConfig, false)
//! session, err := app.CreateSession(config, destination)
//! session.Publish(data, payloadType, metadata)
//! ```

// Module declarations
mod adapter;
mod build_info;
mod client_config;
mod common;
mod common_config;
mod completion_handle;
mod errors;
mod identity_config;
mod message_context;
mod name;
mod runtime;
mod server_config;
mod service;
mod session_context;

// Public re-exports
pub use adapter::{BindingsAdapter, SessionWithCompletion, create_app_with_secret};
pub use build_info::{BuildInfo, get_build_info, get_version};
pub use client_config::{
    BackoffConfig, ClientConfig, ExponentialBackoff, KeepaliveConfig, ProxyConfig,
    new_insecure_client_config,
};
pub use common::initialize_crypto_provider;
pub use common_config::{
    BasicAuth, CaSource, ClientAuthenticationConfig, ServerAuthenticationConfig, SpireConfig,
    TlsClientConfig, TlsServerConfig, TlsSource,
};
pub use completion_handle::CompletionHandle;
pub use errors::SlimError;
pub use identity_config::{
    ClientJwtAuth, IdentityProviderConfig, IdentityVerifierConfig, JwtAlgorithm, JwtAuth,
    JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType, StaticJwtAuth,
};
pub use message_context::{MessageContext, ReceivedMessage};
pub use name::Name;
pub use runtime::get_runtime;
pub use server_config::{
    KeepaliveServerParameters, ServerConfig, new_insecure_server_config, new_server_config,
};
pub use service::{
    DataplaneConfig, Service, ServiceConfiguration, connect, create_service,
    create_service_with_config, disconnect, get_connection_id, get_global_service,
    get_or_init_global_service, new_dataplane_config, new_service_configuration, run_server,
    service_config, service_name, service_run, service_shutdown, stop_server,
};
pub use session_context::{BindingsSessionContext, SessionConfig, SessionType};

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
