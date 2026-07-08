// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! On-disk representation of a session, used to restore it after a restart
//! without repeating the discovery/invite/welcome handshake.
//!
//! The record captures everything needed to rebuild a [`SessionController`] and
//! rejoin its (already-established) MLS group: the session identity and config,
//! the roster, the MLS bookkeeping (`group_id`, `last_mls_msg_id`, and, for a
//! moderator, the participant→identity map and commit counter), and the routing
//! endpoints.
//!
//! The wire types (`Name`, `Participant`) are prost messages without serde
//! support, so they are stored as their prost-encoded bytes inside an otherwise
//! serde/JSON record. App-level, non-portable config (the persistence settings
//! themselves) is deliberately *not* stored — it is re-supplied on restore.

use std::collections::HashMap;

use prost::Message as _;
use serde::{Deserialize, Serialize};
use slim_datapath::api::{Participant, ProtoName, ProtoSessionType};

use crate::errors::SessionError;
use crate::session_config::{MlsSettings, SessionConfig};
use crate::session_layer::Direction;

/// Current schema version, bumped on incompatible record changes so old records
/// can be detected and skipped rather than misparsed.
const SCHEMA_VERSION: u32 = 1;

/// Key prefix under which session records are stored in the KV store.
pub(crate) const SESSION_KEY_PREFIX: &str = "session:";

/// The storage key for a given session id.
pub(crate) fn session_key(session_id: u32) -> String {
    format!("{SESSION_KEY_PREFIX}{session_id}")
}

/// A serialized, restorable snapshot of a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedSession {
    pub version: u32,
    pub session_id: u32,
    /// prost-encoded [`ProtoName`].
    pub source: Vec<u8>,
    pub destination: Vec<u8>,
    pub control: Vec<u8>,
    pub direction: u8,
    pub config: PersistedConfig,
    /// MLS group id, if the session uses MLS (used to `Mls::load`).
    pub group_id: Option<Vec<u8>>,
    /// Highest applied MLS control-message id.
    pub last_mls_msg_id: u32,
    /// Connection id the session is reachable over, if known.
    pub conn_id: Option<u64>,
    pub role: PersistedRole,
}

/// Role-specific state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PersistedRole {
    Moderator {
        /// prost-encoded [`Participant`] entries (each carries its own name).
        group_list: Vec<Vec<u8>>,
        /// (prost-encoded [`ProtoName`], MLS identity bytes) for group removals.
        mls_participants: Vec<(Vec<u8>, Vec<u8>)>,
        /// Next MLS commit id the moderator will assign.
        next_msg_id: u32,
    },
    Participant {
        /// prost-encoded [`ProtoName`] of the moderator, if known.
        moderator_name: Option<Vec<u8>>,
        /// (prost-encoded [`ProtoName`], sends_data, receives_data).
        group_list: Vec<(Vec<u8>, bool, bool)>,
    },
}

/// Portable subset of [`SessionConfig`]. Excludes the persistence settings,
/// which are re-supplied by the layer on restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedConfig {
    pub session_type: i32,
    pub max_retries: Option<u32>,
    pub interval_ms: Option<u64>,
    pub initiator: bool,
    pub metadata: HashMap<String, String>,
    pub mls: Option<PersistedMls>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedMls {
    pub header_integrity_validation_percent: u32,
    pub max_seen_control_message_ids_size: Option<usize>,
}

impl PersistedSession {
    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, SessionError> {
        serde_json::to_vec(self).map_err(SessionError::from)
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, SessionError> {
        let record: PersistedSession = serde_json::from_slice(bytes).map_err(SessionError::from)?;
        if record.version != SCHEMA_VERSION {
            return Err(SessionError::PersistenceSchemaMismatch {
                expected: SCHEMA_VERSION,
                got: record.version,
            });
        }
        Ok(record)
    }
}

// ---------------------------------------------------------------------------
// Encoding helpers for the prost-typed fields.
// ---------------------------------------------------------------------------

pub(crate) fn encode_name(name: &ProtoName) -> Vec<u8> {
    name.encode_to_vec()
}

pub(crate) fn decode_name(bytes: &[u8]) -> Result<ProtoName, SessionError> {
    ProtoName::decode(bytes).map_err(|e| SessionError::PersistenceDecode(e.to_string()))
}

pub(crate) fn encode_participant(p: &Participant) -> Vec<u8> {
    p.encode_to_vec()
}

pub(crate) fn decode_participant(bytes: &[u8]) -> Result<Participant, SessionError> {
    Participant::decode(bytes).map_err(|e| SessionError::PersistenceDecode(e.to_string()))
}

// ---------------------------------------------------------------------------
// Config conversions.
// ---------------------------------------------------------------------------

impl PersistedConfig {
    pub(crate) fn from_config(config: &SessionConfig) -> Self {
        PersistedConfig {
            session_type: config.session_type as i32,
            max_retries: config.max_retries,
            interval_ms: config.interval.map(|d| d.as_millis() as u64),
            initiator: config.initiator,
            metadata: config.metadata.clone(),
            mls: config.mls_settings.as_ref().map(|m| PersistedMls {
                header_integrity_validation_percent: m.header_integrity_validation_percent,
                max_seen_control_message_ids_size: m.max_seen_control_message_ids_size,
            }),
        }
    }

    /// Rebuild a [`SessionConfig`]. Persistence is app-level (supplied by the
    /// session layer), so it is not part of the restored config.
    pub(crate) fn into_config(self) -> SessionConfig {
        SessionConfig {
            session_type: ProtoSessionType::try_from(self.session_type)
                .unwrap_or(ProtoSessionType::Unspecified),
            max_retries: self.max_retries,
            interval: self.interval_ms.map(std::time::Duration::from_millis),
            mls_settings: self.mls.map(|m| MlsSettings {
                header_integrity_validation_percent: m.header_integrity_validation_percent,
                max_seen_control_message_ids_size: m.max_seen_control_message_ids_size,
            }),
            initiator: self.initiator,
            metadata: self.metadata,
        }
    }
}

pub(crate) fn encode_direction(direction: Direction) -> u8 {
    match direction {
        Direction::Send => 0,
        Direction::Recv => 1,
        Direction::Bidirectional => 2,
        Direction::None => 3,
    }
}

pub(crate) fn decode_direction(value: u8) -> Direction {
    match value {
        0 => Direction::Send,
        1 => Direction::Recv,
        3 => Direction::None,
        _ => Direction::Bidirectional,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn new_record(
    session_id: u32,
    source: &ProtoName,
    destination: &ProtoName,
    control: &ProtoName,
    direction: Direction,
    config: &SessionConfig,
    group_id: Option<Vec<u8>>,
    last_mls_msg_id: u32,
    conn_id: Option<u64>,
    role: PersistedRole,
) -> PersistedSession {
    PersistedSession {
        version: SCHEMA_VERSION,
        session_id,
        source: encode_name(source),
        destination: encode_name(destination),
        control: encode_name(control),
        direction: encode_direction(direction),
        config: PersistedConfig::from_config(config),
        group_id,
        last_mls_msg_id,
        conn_id,
        role,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::api::ParticipantSettings;

    fn name(leaf: &str) -> ProtoName {
        ProtoName::from_strings(["org", "ns", leaf])
    }

    #[test]
    fn moderator_record_roundtrips() {
        let source = name("mod").with_id(1);
        let dest = name("channel");
        let p1 = Participant::new(
            name("alice").with_id(2),
            ParticipantSettings::bidirectional(),
        );

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(5),
            interval: Some(std::time::Duration::from_millis(250)),
            mls_settings: Some(MlsSettings {
                header_integrity_validation_percent: 100,
                max_seen_control_message_ids_size: Some(64),
            }),
            initiator: true,
            metadata: HashMap::from([("k".to_string(), "v".to_string())]),
        };

        let record = new_record(
            42,
            &source,
            &dest,
            &dest,
            Direction::Bidirectional,
            &config,
            Some(vec![9, 9, 9]),
            7,
            Some(3),
            PersistedRole::Moderator {
                group_list: vec![encode_participant(&p1)],
                mls_participants: vec![(encode_name(&name("alice").with_id(2)), vec![1, 2, 3])],
                next_msg_id: 4,
            },
        );

        let bytes = record.to_bytes().unwrap();
        let back = PersistedSession::from_bytes(&bytes).unwrap();

        assert_eq!(back.session_id, 42);
        assert_eq!(decode_name(&back.source).unwrap(), source);
        assert_eq!(back.group_id, Some(vec![9, 9, 9]));
        assert_eq!(back.last_mls_msg_id, 7);
        assert_eq!(back.conn_id, Some(3));

        match back.role {
            PersistedRole::Moderator {
                group_list,
                mls_participants,
                next_msg_id,
            } => {
                assert_eq!(next_msg_id, 4);
                assert_eq!(decode_participant(&group_list[0]).unwrap(), p1);
                assert_eq!(mls_participants[0].1, vec![1, 2, 3]);
            }
            _ => panic!("expected moderator"),
        }

        let cfg = back.config.into_config();
        assert_eq!(cfg.session_type, ProtoSessionType::Multicast);
        assert_eq!(cfg.max_retries, Some(5));
        assert_eq!(cfg.interval, Some(std::time::Duration::from_millis(250)));
        assert!(cfg.initiator);
        assert!(cfg.mls_settings.is_some());
    }

    #[test]
    fn participant_record_roundtrips() {
        let source = name("bob").with_id(5);
        let dest = name("channel");
        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            initiator: false,
            ..Default::default()
        };

        let record = new_record(
            1,
            &source,
            &dest,
            &dest,
            Direction::Recv,
            &config,
            None,
            0,
            None,
            PersistedRole::Participant {
                moderator_name: Some(encode_name(&name("mod").with_id(1))),
                group_list: vec![(encode_name(&name("alice").with_id(2)), true, false)],
            },
        );

        let back = PersistedSession::from_bytes(&record.to_bytes().unwrap()).unwrap();
        assert_eq!(decode_direction(back.direction), Direction::Recv);
        match back.role {
            PersistedRole::Participant {
                moderator_name,
                group_list,
            } => {
                assert_eq!(
                    decode_name(&moderator_name.unwrap()).unwrap(),
                    name("mod").with_id(1)
                );
                assert!(group_list[0].1);
                assert!(!group_list[0].2);
            }
            _ => panic!("expected participant"),
        }
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let source = name("x");
        let config = SessionConfig::default();
        let mut record = new_record(
            1,
            &source,
            &source,
            &source,
            Direction::Bidirectional,
            &config,
            None,
            0,
            None,
            PersistedRole::Participant {
                moderator_name: None,
                group_list: vec![],
            },
        );
        record.version = 999;
        assert!(PersistedSession::from_bytes(&record.to_bytes().unwrap()).is_err());
    }
}
