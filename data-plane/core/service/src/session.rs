// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod channel_endpoint;
mod common;
mod config;
pub mod context;
mod errors;
pub mod interceptor;
pub mod interceptor_mls;
mod message;
mod moderator_task;
pub mod multicast;
pub mod notification;
pub mod point_to_point;
pub mod producer_buffer;
pub mod receiver_buffer;
mod session_layer;
pub mod timer;
mod traits;
pub mod transmitter;

// Traits
pub use traits::Transmitter;
pub(crate) use traits::{CommonSession, MessageHandler, SessionConfigTrait};

// Common types that session modules need
pub(crate) use common::{Common, State};

// Session Info
pub use message::Id;

// Session Errors
pub use errors::SessionError;

// Interceptor
pub use interceptor::SessionInterceptorProvider;

// Session Config
pub use config::SessionConfig;

// Common Session Types - internal use
pub(crate) use common::{MessageDirection, SESSION_RANGE, Session, SlimChannelSender};

// Session layer
pub(crate) use session_layer::SessionLayer;
// Public exports for external crates (like Python bindings)
pub use common::{SESSION_UNSPECIFIED, SessionType, AppChannelReceiver};

// Re-export specific items that need to be publicly accessible
pub use multicast::MulticastConfiguration;
pub use point_to_point::PointToPointConfiguration;
