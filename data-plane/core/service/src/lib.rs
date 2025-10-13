// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SLIM Service - Main service and public API to interact with SLIM data plane.
//!
//! This crate provides the core service functionality for SLIM, including:
//!
//! - **Service**: Main service component that manages message processing and connections
//! - **App**: Application-level API for session management and messaging
//! - **AppAdapter**: Bridge between App API and language bindings (e.g., PyService)
//!
//! ## Basic Usage
//!
//! ```rust
//! # tokio_test::block_on(async {
//! use slim_service::Service;
//! use slim_config::component::ComponentBuilder;
//! use slim_service::bindings::AppAdapter;
//! use slim_auth::shared_secret::SharedSecret;
//! use slim_datapath::messages::Name;
//!
//! // Create service instance (handles message processing)
//! let service = Service::builder().build("svc-0".to_string()).expect("Failed to create service");
//!
//! // Create authentication components
//! let provider = SharedSecret::new("myapp", "my_secret");
//! let verifier = SharedSecret::new("myapp", "my_secret");
//!
//! // Create adapter for language bindings - Method 1: Direct creation
//! let app_name = Name::from_strings(["org", "ns", "app"]);
//! let adapter = AppAdapter::new_with_service(
//!     &service,
//!     app_name,
//!     provider.clone(),
//!     verifier.clone(),
//! ).await.expect("Failed to create AppAdapter");
//!
//! // Alternative - Method 2: Builder pattern
//! let adapter2 = AppAdapter::builder()
//!    .with_name(Name::from_strings(["my", "app2", "v1"]))
//!    .with_identity_provider(provider)
//!    .with_identity_verifier(verifier)
//!    .build(&service).await.expect("Failed to create AppAdapter via builder");
//! # })
//! ```
//!
//! ## AppAdapter Integration
//!
//! The `AppAdapter` requires a `Service` instance to properly create `App` instances.
//! The service handles:
//!
//! - Message processing and routing
//! - Connection registration and management
//! - Storage path creation and management
//! - Resource lifecycle and cleanup
//!
//! **Note**: Always create `AppAdapter` instances using `Service::create_app()` through
//! `AppAdapter::create()` or `AppAdapter::builder().build()`.
//!
//! Use `Service::builder().build()` to create Service instances easily.

pub mod errors;
#[macro_use]
pub mod service;

pub mod app;
pub mod bindings;

// Third-party crates
pub use slim_datapath::messages::utils::SlimHeaderFlags;

// Local crate
pub use bindings::{AppAdapter, AppAdapterBuilder};
pub use errors::ServiceError;
pub use service::{KIND, Service, ServiceBuilder, ServiceConfiguration};
