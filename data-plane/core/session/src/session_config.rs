// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use slim_datapath::api::{CommandPayload, ProtoSessionType};

use crate::SessionError;

#[derive(Default, Clone)]
pub struct SessionConfig {
    /// session type
    pub session_type: ProtoSessionType,

    /// number of retries for each message/rtx
    pub max_retries: Option<u32>,

    /// time between retries
    pub duration: Option<std::time::Duration>,

    /// true is mls is enabled
    pub mls_enabled: bool,

    /// true is the local endpoint is initiator of the session
    pub initiator: bool,

    /// metadata related to the sessions
    pub metadata: HashMap<String, String>,
}

impl SessionConfig {
    pub fn with_session_type(&self, session_type: ProtoSessionType) -> Self {
        Self {
            session_type,
            max_retries: self.max_retries,
            duration: self.duration,
            mls_enabled: self.mls_enabled,
            initiator: self.initiator,
            metadata: self.metadata.clone(),
        }
    }

    pub fn from_join_request(
        session_type: ProtoSessionType,
        payload: &CommandPayload,
        metadata: HashMap<String, String>,
        initiator: bool,
    ) -> Result<Self, SessionError> {
        let join = payload.as_join_request_payload().map_err(|e| {
            SessionError::Processing(format!("failed to get join request payload: {}", e))
        })?;
        let (duration, max_retries) = if let Some(ts) = &join.timer_settings {
            (
                Some(std::time::Duration::from_millis(ts.timeout as u64)),
                Some(ts.max_retries),
            )
        } else {
            (None, None)
        };

        Ok(SessionConfig {
            session_type,
            max_retries,
            duration,
            mls_enabled: join.enable_mls,
            initiator,
            metadata,
        })
    }
}
