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
pub mod adapter;
pub mod common;
mod message_context;
pub mod name;
pub mod runtime;
mod service_ref;
pub mod session_context;

pub use name::Name;

// Public re-exports
pub use adapter::{
    BindingsAdapter, BuildInfo, ClientConfig, FfiCompletionHandle, ReceivedMessage, ServerConfig,
    SessionConfig, SessionType, SlimError, TlsConfig, create_app_with_secret, get_build_info,
    get_version,
};
pub use common::initialize_crypto_provider;
pub use message_context::MessageContext;
pub use name::Name;
pub use runtime::get_runtime;
pub use service_ref::{ServiceRef, get_or_init_global_service};
pub use session_context::BindingsSessionContext;

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
