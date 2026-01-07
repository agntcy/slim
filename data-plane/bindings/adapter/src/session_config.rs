// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::api::ProtoSessionType;
use slim_session::SessionConfig as SlimSessionConfig;

/// Session type enum
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SessionType {
    PointToPoint,
    Group,
}

/// Session configuration
#[derive(uniffi::Record)]
pub struct SessionConfig {
    /// Session type (PointToPoint or Group)
    pub session_type: SessionType,

    /// Enable MLS encryption for this session
    pub enable_mls: bool,

    /// Maximum number of retries for message transmission (None = use default)
    pub max_retries: Option<u32>,

    /// Interval between retries in milliseconds (None = use default)
    pub interval_ms: Option<u64>,

    /// Custom metadata key-value pairs for the session
    pub metadata: std::collections::HashMap<String, String>,
}

impl From<SessionConfig> for SlimSessionConfig {
    fn from(config: SessionConfig) -> Self {
        SlimSessionConfig {
            session_type: match config.session_type {
                SessionType::PointToPoint => ProtoSessionType::PointToPoint,
                SessionType::Group => ProtoSessionType::Multicast,
            },
            max_retries: config.max_retries,
            interval: config.interval_ms.map(std::time::Duration::from_millis),
            mls_enabled: config.enable_mls,
            initiator: false,
            metadata: config.metadata,
        }
    }
}
