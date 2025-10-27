// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Bindings adapter that bridges the core App API with language-bindings interface
//!
//! This module provides an adapter layer that converts between the core App API
//! and the interface expected by bindings, handling type conversions and
//! API differences.
//!
//! # Architecture
//!
//! The module is organized into distinct components with clear separation of concerns:
//!
//! - **`BindingsAdapter`**: App-level operations (creation, configuration, session management)
//! - **`BindingsSessionContext`**: Session-specific operations (publish, invite, remove, message reception)
//! - **`MessageContext`**: Message metadata and routing information
//! - **`AppAdapterBuilder`**: Flexible configuration and construction
//! - **`ServiceRef`**: Service reference management (global vs local)
//!
//! # Usage
//!
//! ## Basic Usage
//!
//! ```rust
//! # tokio_test::block_on(async {
//! use slim_service::{Service, ServiceBuilder};
//! use slim_config::component::ComponentBuilder;
//! use slim_service::bindings::{BindingsAdapter, BindingsSessionContext};
//! use slim_auth::shared_secret::SharedSecret;
//! use slim_session::{SessionConfig, point_to_point::PointToPointConfiguration};
//! use slim_datapath::messages::Name;
//!
//! // Create authentication components
//! use slim_auth::testutils::TEST_VALID_SECRET;
//! let provider = SharedSecret::new("myapp", TEST_VALID_SECRET);
//! let verifier = SharedSecret::new("myapp", TEST_VALID_SECRET);
//!
//! // Create adapter with complete setup
//! let base_name = Name::from_strings(["org", "ns", "svc"]);
//! let (adapter, _service_ref) = BindingsAdapter::new(
//!     base_name,
//!     provider,
//!     verifier,
//!     false, // use global service
//! ).await.expect("failed to create adapter");
//!
//! // Create a session
//! let session_config = SessionConfig::PointToPoint(PointToPointConfiguration::default());
//! let session_ctx = adapter.create_session(session_config).await
//!     .expect("failed to create session");
//! let session_bindings = BindingsSessionContext::from(session_ctx);
//!
//! // Use session for operations
//! let name = Name::from_strings(["org", "ns", "service"]);
//! session_bindings.publish(&name, 1, b"hello".to_vec(), None, None, None).await
//!     .expect("failed to publish message");
//!# })
//! ```
//!
//! ## Construction Methods
//!
//! The adapter can be created in four ways:
//!
//! 1. **Complete Creation**: `BindingsAdapter::new()` - Full creation with service management, token validation and random ID (recommended for language bindings)
//! 2. **With Service**: `BindingsAdapter::new_with_service()` - Creates adapter from existing service and explicit name
//! 3. **With App**: `BindingsAdapter::new_with_app()` - Creates adapter from existing app instance
//! 4. **Builder Pattern**: `BindingsAdapter::builder()` or `AppAdapterBuilder::new()` - More flexible configuration
//!
//! ## Separation of Concerns
//!
//! ### BindingsAdapter (App-Level Operations)
//! - Service management and configuration
//! - Session lifecycle (create/delete/listen)
//! - App-level routing (subscribe/unsubscribe/routes)
//! - Identity and authentication management
//!
//! ### BindingsSessionContext (Session-Level Operations)
//! - Message publishing (publish/publish_to)
//! - Participant management (invite/remove)
//! - Message reception (get_session_message)
//! - Session-specific operations
//!
//! ## Message Reception Architecture
//!
//! SLIM uses a two-level message reception system:
//!
//! 1. **App-level notifications** (`adapter.notification_rx`): Receives session lifecycle events
//!    - `Notification::NewSession` - When new sessions are established
//!    - Used by `adapter.listen_for_session()` to detect incoming connections
//!
//! 2. **Session-level messages** (`session_context.rx`): Receives actual message content
//!    - Individual `ProtoMessage` instances with application payloads
//!    - Used by `session_context.get_session_message()` to receive messages from specific sessions
//!
//! **Important**: All message content reception is session-specific. There is no
//! app-level message listening - each session has its own message channel.
//!
//! ## Features
//!
//! - **Complete Service Management**: Global/local service creation and management for language bindings
//! - **Clear Separation**: App-level vs session-level operations are clearly separated
//! - **Session Operations**: Publish, invite, remove, and message reception on session contexts
//! - **Route Management**: Set up communication paths between endpoints
//! - **Message Handling**: Generic message context with routing and metadata for language bindings
//! - **Authentication**: Token validation and identity management
//! - **Async Operations**: Full async/await support for all operations

// Module declarations
mod adapter;
mod builder;
mod message_context;
mod service_ref;
mod session_context;

// Public re-exports
pub use adapter::BindingsAdapter;
pub use builder::AppAdapterBuilder;
pub use message_context::MessageContext;
pub use service_ref::{ServiceRef, get_or_init_global_service};
pub use session_context::BindingsSessionContext;
