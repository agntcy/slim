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
mod identity;
mod message_context;
mod service_ref;
pub mod session_context;

// Public re-exports
pub use adapter::{
    BindingsAdapter, BuildInfo, CaSource, ClientConfig, FfiCompletionHandle, Name,
    ReceivedMessage, ServerConfig, SessionConfig, SessionType, SlimError, TlsConfig, TlsSource,
    create_app, create_app_local_svc, create_app_secret_local_svc, create_app_with_secret,
    get_build_info, get_runtime, get_version, initialize_crypto_provider,
};
pub use identity::{
    BindingsAlgorithm, BindingsIdentityProvider, BindingsIdentityVerifier, BindingsKey,
    BindingsKeyData, BindingsKeyFormat, IdentityError, create_identity_provider_jwt,
    create_identity_provider_shared_secret, create_identity_provider_spire,
    create_identity_provider_static_jwt, create_identity_verifier_jwt,
    create_identity_verifier_shared_secret, create_identity_verifier_spire, create_key_with_jwks,
};
pub use message_context::MessageContext;
pub use service_ref::{ServiceRef, get_or_init_global_service};
pub use session_context::BindingsSessionContext;

// UniFFI scaffolding setup (must be at crate root)
uniffi::setup_scaffolding!();
