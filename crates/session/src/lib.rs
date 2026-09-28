// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod common;
pub mod completion_handle;
pub mod context;
pub mod controller_sender;
pub mod errors;

pub mod mls_state;
mod moderator_task;
pub mod notification;
mod persistence;
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
pub mod session_settings;
pub mod subscription_manager;
pub mod timer;
pub mod timer_factory;
pub mod traits;

// Runtime-agnostic helpers (await seam for the MLS sync/async split) shared by
// the modules above so the same source compiles for native tokio and the wasm32
// browser runtime. Imported by path: `use crate::runtime::maybe_await;`.
mod runtime;

// Test utilities (only available during tests)
#[cfg(test)]
pub mod test_utils;

/// Fuzzing-only surface for `crates/session/fuzz` (`agntcy-slim-session-fuzz`).
///
/// `decode_name`/`decode_participant` are `pub(crate)` in [`persistence`]
/// because they are an internal parsing step, not public API. This module
/// exists only behind the `fuzzing` feature (off by default, not part of any
/// default feature set) so the fuzz crate can call them without making them
/// generally `pub`; `#[doc(hidden)]` also keeps it out of rustdoc for anyone
/// who does enable the feature. Not covered by semver.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing {
    pub use crate::errors::SessionError;
    pub use crate::persistence::decode_name_fuzz as decode_name;
    pub use crate::persistence::decode_participant_fuzz as decode_participant;
}

// Traits
pub use traits::MessageHandler;

// Re-export the unified builder for convenience
pub use session_builder::{ForController, ForModerator, ForParticipant, SessionBuilder};

// Session Errors
pub use errors::SessionError;

// Session Config
pub use session_config::SessionConfig;

// Session Layer
pub use session_layer::{Direction, SessionLayer};

// Common Session Types - internal use
pub use common::{MessageDirection, SESSION_RANGE, SessionMessage, SlimChannelSender};

// Session output types
pub use common::{OutboundMessage, SessionOutput};

// Public exports for external crates (like Python bindings)
pub use common::{AppChannelReceiver, SESSION_UNSPECIFIED};

// Re-export specific items that need to be publicly accessible
pub use completion_handle::CompletionHandle;
pub use notification::Notification;
pub use session_controller::CloseMode;
pub use subscription_manager::{AutoAckManager, SubscriptionOps};
