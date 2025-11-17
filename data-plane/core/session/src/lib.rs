// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod common;
pub mod completion_handle;
pub mod context;
pub mod controller_sender;
mod errors;
pub mod interceptor;
pub mod interceptor_mls;
mod mls_state;
mod moderator_task;
pub mod notification;
pub mod producer_buffer;
pub mod receiver_buffer;
pub mod session;
mod session_builder;
pub mod session_config;
pub mod session_controller;
mod session_layer;
mod session_moderator;
mod session_participant;
pub mod session_receiver;
pub mod session_sender;
mod session_settings;
pub mod timer;
pub mod timer_factory;
pub mod traits;
pub mod transmitter;

// Test utilities (only available during tests)
#[cfg(test)]
pub mod test_utils;

// Traits
pub use traits::{MessageHandler, Transmitter};

// Re-export the unified builder for convenience
pub use session_builder::{ForController, ForModerator, ForParticipant, SessionBuilder};

// Session Errors
pub use errors::SessionError;

// Interceptor
pub use interceptor::SessionInterceptorProvider;

// Session Config
pub use session_config::SessionConfig;

// Common Session Types - internal use
pub use common::{MessageDirection, SESSION_RANGE, SessionMessage, SlimChannelSender};

// Session layer
pub use session_layer::SessionLayer;
// Public exports for external crates (like Python bindings)
pub use common::{AppChannelReceiver, SESSION_UNSPECIFIED};

// Re-export specific items that need to be publicly accessible
pub use completion_handle::CompletionHandle;
pub use notification::Notification;
