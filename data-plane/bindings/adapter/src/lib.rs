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
//! use slim_bindings::{create_app_with_secret, Name, SessionConfig, SessionType};
//!
//! // Create an app
//! let app_name = Name { components: vec!["org".into(), "app".into(), "v1".into()], id: None };
//! let app = create_app_with_secret(app_name, "my-secret".to_string())?;
//!
//! // Create a session
//! let config = SessionConfig { session_type: SessionType::PointToPoint, ... };
//! let session = app.create_session(config, destination)?;
//! ```
//!
//! ### From Go (via generated bindings)
//!
//! ```go
//! app, err := slim.CreateAppWithSecret(appName, sharedSecret)
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
mod message_context;
mod name;
mod runtime;
mod server_config;
mod service_ref;
mod session_context;

// Public re-exports
pub use adapter::{BindingsAdapter, FfiCompletionHandle, SessionWithCompletion, create_app_with_secret};
pub use completion_handle::CompletionHandle;
pub use build_info::{BuildInfo, get_build_info, get_version};
pub use client_config::{ClientConfig, new_insecure_client_config};
pub use common::initialize_crypto_provider;
pub use common_config::{
    BasicAuth, CaSource, ClientAuthenticationConfig, ClientJwtAuth, JwtAlgorithm, JwtAuth,
    JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType, ServerAuthenticationConfig, SpireConfig,
    StaticJwtAuth, TlsClientConfig, TlsServerConfig, TlsSource,
};
pub use errors::SlimError;
pub use message_context::{MessageContext, ReceivedMessage};
pub use name::Name;
pub use runtime::get_runtime;
pub use server_config::{ServerConfig, new_insecure_server_config, new_server_config};
pub use service_ref::{ServiceRef, get_or_init_global_service};
pub use session_context::{BindingsSessionContext, SessionConfig, SessionType};

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
