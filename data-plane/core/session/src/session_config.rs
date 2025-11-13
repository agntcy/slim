// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use slim_datapath::api::{CommandPayload, ProtoSessionType};

use crate::{SessionError, timer_factory::TimerSettings};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct SessionConfig {
    /// session type
    pub session_type: ProtoSessionType,

    /// number of retries for each message/rtx
    pub max_retries: Option<u32>,

    /// interval between retries
    pub interval: Option<std::time::Duration>,

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
            interval: self.interval,
            mls_enabled: self.mls_enabled,
            initiator: self.initiator,
            metadata: self.metadata.clone(),
        }
    }

    pub fn get_timer_settings(&self) -> TimerSettings {
        TimerSettings::constant(self.interval.unwrap_or(std::time::Duration::from_secs(1)))
            .with_max_retries(self.max_retries.unwrap_or(10))
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
            interval: duration,
            mls_enabled: join.enable_mls,
            initiator,
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::api::CommandPayload;
    use slim_datapath::messages::Name;
    use std::time::Duration;

    #[test]
    fn test_default() {
        let config = SessionConfig::default();
        assert_eq!(config.session_type, ProtoSessionType::Unspecified);
        assert_eq!(config.max_retries, None);
        assert_eq!(config.interval, None);
        assert!(!config.mls_enabled);
        assert!(!config.initiator);
        assert!(config.metadata.is_empty());
    }

    #[test]
    fn test_with_session_type() {
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());

        let config = SessionConfig {
            session_type: ProtoSessionType::Unspecified,
            max_retries: Some(5),
            interval: Some(Duration::from_secs(10)),
            mls_enabled: true,
            initiator: true,
            metadata: metadata.clone(),
        };

        let new_config = config.with_session_type(ProtoSessionType::Multicast);

        assert_eq!(new_config.session_type, ProtoSessionType::Multicast);
        assert_eq!(new_config.max_retries, Some(5));
        assert_eq!(new_config.interval, Some(Duration::from_secs(10)));
        assert!(new_config.mls_enabled);
        assert!(new_config.initiator);
        assert_eq!(new_config.metadata.len(), 1);
        assert_eq!(new_config.metadata.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_with_session_type_point_to_point() {
        let config = SessionConfig::default();
        let new_config = config.with_session_type(ProtoSessionType::PointToPoint);
        assert_eq!(new_config.session_type, ProtoSessionType::PointToPoint);
    }

    #[test]
    fn test_from_join_request_with_timer_settings() {
        let dest = Name::from_strings(["dest", "", ""]);
        let payload = CommandPayload::builder().join_request(
            true,
            Some(3),
            Some(Duration::from_millis(500)),
            Some(dest),
        );

        let mut metadata = HashMap::new();
        metadata.insert("test_key".to_string(), "test_value".to_string());

        let config = SessionConfig::from_join_request(
            ProtoSessionType::Multicast,
            &payload,
            metadata.clone(),
            true,
        )
        .unwrap();

        assert_eq!(config.session_type, ProtoSessionType::Multicast);
        assert_eq!(config.max_retries, Some(3));
        assert_eq!(config.interval, Some(Duration::from_millis(500)));
        assert!(config.mls_enabled);
        assert!(config.initiator);
        assert_eq!(config.metadata.len(), 1);
        assert_eq!(
            config.metadata.get("test_key"),
            Some(&"test_value".to_string())
        );
    }

    #[test]
    fn test_from_join_request_without_timer_settings() {
        let dest = Name::from_strings(["dest", "", ""]);
        let payload = CommandPayload::builder().join_request(false, None, None, Some(dest));

        let metadata = HashMap::new();

        let config = SessionConfig::from_join_request(
            ProtoSessionType::PointToPoint,
            &payload,
            metadata,
            false,
        )
        .unwrap();

        assert_eq!(config.session_type, ProtoSessionType::PointToPoint);
        assert_eq!(config.max_retries, None);
        assert_eq!(config.interval, None);
        assert!(!config.mls_enabled);
        assert!(!config.initiator);
        assert!(config.metadata.is_empty());
    }

    #[test]
    fn test_from_join_request_with_mls_enabled() {
        let dest = Name::from_strings(["dest", "", ""]);
        let payload = CommandPayload::builder().join_request(
            true,
            Some(10),
            Some(Duration::from_secs(5)),
            Some(dest),
        );

        let config = SessionConfig::from_join_request(
            ProtoSessionType::Multicast,
            &payload,
            HashMap::new(),
            false,
        )
        .unwrap();

        assert!(config.mls_enabled);
    }

    #[test]
    fn test_from_join_request_invalid_payload() {
        // Create a payload that is not a join request
        let dest = Name::from_strings(["dest", "", ""]);
        let payload = CommandPayload::builder().leave_request(Some(dest));

        let result = SessionConfig::from_join_request(
            ProtoSessionType::Multicast,
            &payload,
            HashMap::new(),
            true,
        );

        assert!(result.is_err());
        if let Err(SessionError::Processing(msg)) = result {
            assert!(msg.contains("failed to get join request payload"));
        } else {
            panic!("Expected SessionError::Processing");
        }
    }

    #[test]
    fn test_clone() {
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(7),
            interval: Some(Duration::from_millis(1000)),
            mls_enabled: true,
            initiator: false,
            metadata: metadata.clone(),
        };

        let cloned = config.clone();

        assert_eq!(cloned.session_type, config.session_type);
        assert_eq!(cloned.max_retries, config.max_retries);
        assert_eq!(cloned.interval, config.interval);
        assert_eq!(cloned.mls_enabled, config.mls_enabled);
        assert_eq!(cloned.initiator, config.initiator);
        assert_eq!(cloned.metadata, config.metadata);
    }

    #[test]
    fn test_from_join_request_with_large_timeout() {
        let dest = Name::from_strings(["dest", "", ""]);
        let payload = CommandPayload::builder().join_request(
            true,
            Some(100),
            Some(Duration::from_secs(3600)), // 1 hour
            Some(dest),
        );

        let config = SessionConfig::from_join_request(
            ProtoSessionType::Multicast,
            &payload,
            HashMap::new(),
            true,
        )
        .unwrap();

        assert_eq!(config.max_retries, Some(100));
        assert_eq!(config.interval, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn test_metadata_preservation() {
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());
        metadata.insert("key3".to_string(), "value3".to_string());

        let config = SessionConfig {
            session_type: ProtoSessionType::Unspecified,
            max_retries: None,
            interval: None,
            mls_enabled: false,
            initiator: false,
            metadata: metadata.clone(),
        };

        let new_config = config.with_session_type(ProtoSessionType::Multicast);

        assert_eq!(new_config.metadata.len(), 3);
        assert_eq!(new_config.metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(new_config.metadata.get("key2"), Some(&"value2".to_string()));
        assert_eq!(new_config.metadata.get("key3"), Some(&"value3".to_string()));
    }
}
