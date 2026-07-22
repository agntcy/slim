// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SLIM Service - Main service and public API to interact with SLIM data plane.
//!
//! This crate provides the core service functionality for SLIM, including:
//!
//! - **Service**: Main service component that manages message processing and connections
//! - **App**: Application-level API for session management and messaging
//!
//! For language bindings (Go, Python, etc.), see the `slim_bindings` crate.
//!
//! ## Basic Usage
//!
//! ```rust
//! # tokio_test::block_on(async {
//! use slim_service::Service;
//! use slim_config::component::ComponentBuilder;
//! use slim_auth::shared_secret::SharedSecret;
//! use slim_datapath::api::ProtoName;
//!
//! // Create service instance (handles message processing)
//! let service = Service::builder().build("svc-0".to_string()).expect("Failed to create service");
//!
//! // Create authentication components
//! let secret = "test-shared-secret-value-0123456789abcdef";
//! let provider = SharedSecret::new("myapp", secret).unwrap();
//! let verifier = SharedSecret::new("myapp", secret).unwrap();
//!
//! // Create an app for messaging
//! let app_name = ProtoName::from_strings(["org", "ns", "app"]);
//! let (app, rx) = service.create_app(&app_name, provider, verifier).expect("Failed to create app");
//! # })
//! ```

pub mod errors;

#[cfg(not(target_arch = "wasm32"))]
pub mod service;

#[cfg(feature = "session")]
pub mod app;

// Third-party crates
pub use slim_datapath::messages::utils::SlimHeaderFlags;

// Local crate
pub use errors::ServiceError;
#[cfg(feature = "session")]
pub use errors::SubscriptionAckError;
#[cfg(not(target_arch = "wasm32"))]
pub use service::{KIND, Service, ServiceBuilder, ServiceConfiguration};
