// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC bindings for data plane service.
pub mod proto;

use crate::errors::DataPathError;
use crate::messages::encoder::calculate_hash;

pub use proto::dataplane::v1::ApplicationPayload;
pub use proto::dataplane::v1::CommandPayload;
pub use proto::dataplane::v1::Content;
pub use proto::dataplane::v1::DiscoveryReplyPayload;
pub use proto::dataplane::v1::DiscoveryRequestPayload;
pub use proto::dataplane::v1::EncodedName;
pub use proto::dataplane::v1::GroupAckPayload;
pub use proto::dataplane::v1::GroupAddPayload;
pub use proto::dataplane::v1::GroupNackPayload;
pub use proto::dataplane::v1::GroupProposalPayload;
pub use proto::dataplane::v1::GroupRemovePayload;
pub use proto::dataplane::v1::GroupWelcomePayload;
pub use proto::dataplane::v1::HeaderIntegrityAad;
pub use proto::dataplane::v1::JoinReplyPayload;
pub use proto::dataplane::v1::JoinRequestPayload;
pub use proto::dataplane::v1::LeaveReplyPayload;
pub use proto::dataplane::v1::LeaveRequestPayload;
pub use proto::dataplane::v1::Link as ProtoLink;
pub use proto::dataplane::v1::LinkNegotiationPayload;
pub use proto::dataplane::v1::Message as ProtoMessage;
pub use proto::dataplane::v1::MlsPayload;
pub use proto::dataplane::v1::MlsSettings as ProtoMlsSettings;
pub use proto::dataplane::v1::Name as ProtoName;
pub use proto::dataplane::v1::NameId;
pub use proto::dataplane::v1::Participant;
pub use proto::dataplane::v1::ParticipantSettings;
pub use proto::dataplane::v1::Publish as ProtoPublish;
pub use proto::dataplane::v1::SessionHeader;
pub use proto::dataplane::v1::SessionMessageType as ProtoSessionMessageType;
pub use proto::dataplane::v1::SessionType as ProtoSessionType;
pub use proto::dataplane::v1::SlimHeader;
pub use proto::dataplane::v1::StringName;
pub use proto::dataplane::v1::Subscribe as ProtoSubscribe;
pub use proto::dataplane::v1::SubscriptionAck as ProtoSubscriptionAck;
pub use proto::dataplane::v1::Unsubscribe as ProtoUnsubscribe;
pub use proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
pub use proto::dataplane::v1::data_plane_service_server::DataPlaneServiceServer;
pub use proto::dataplane::v1::link::LinkType as ProtoLinkType;
pub use proto::dataplane::v1::message::MessageType;
pub use proto::dataplane::v1::message::MessageType::Link as ProtoLinkMessageType;
pub use proto::dataplane::v1::message::MessageType::Publish as ProtoPublishType;
pub use proto::dataplane::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use proto::dataplane::v1::message::MessageType::SubscriptionAck as ProtoSubscriptionAckType;
pub use proto::dataplane::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;

impl NameId {
    // NULL_COMPONENT is used to represent an id component that is not set
    pub const NULL_COMPONENT: u128 = u128::MAX;
    // Channels gets two different names: one for data and one for control
    // messages. The first 3 components are the same for both. Only the 4th
    // component (id) is different.
    // DATA_CHANNEL_ID is the id for the data channel name
    pub const DATA_CHANNEL_ID: u128 = u128::MAX - 2; // ends with 0xfd (data)
    // CONTROL_CHANNEL_ID is the id for the control channel name.
    pub const CONTROL_CHANNEL_ID: u128 = u128::MAX - 3; // ends with 0xfc (control))
    // RESERVED_IDS is the number of reserved IDs [u128::MAX - RESERVED_IDS, u128::MAX)
    // that are not valid for user-defined names.
    pub const RESERVED_IDS: u128 = 50;

    /// Returns true if `id` is one of the reserved values
    /// At the moment we are using only Self::NULL_COMPONENT,
    /// Self::DATA_CHANNEL_ID and Self::CONTROL_CHANNEL_IDm but we
    /// reserve RESERVED_IDS values for future use.
    pub const fn is_reserved_id(id: u128) -> bool {
        id >= (u128::MAX - Self::RESERVED_IDS)
    }
}

impl std::fmt::Display for NameId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: String = (*self).into();
        write!(f, "{}", s)
    }
}

impl From<u128> for NameId {
    fn from(id: u128) -> Self {
        NameId {
            id_0: (id >> 64) as u64,
            id_1: (id & 0xFFFFFFFFFFFFFFFF) as u64,
        }
    }
}

impl From<NameId> for u128 {
    fn from(name_id: NameId) -> Self {
        (name_id.id_0 as u128) << 64 | (name_id.id_1 as u128)
    }
}

impl TryFrom<String> for NameId {
    type Error = DataPathError;

    fn try_from(s: String) -> Result<Self, DataPathError> {
        match s.as_str() {
            "NULL_COMPONENT" => Ok(Self::from(Self::NULL_COMPONENT)),
            "DATA_CHANNEL_ID" => Ok(Self::from(Self::DATA_CHANNEL_ID)),
            "CONTROL_CHANNEL_ID" => Ok(Self::from(Self::CONTROL_CHANNEL_ID)),
            _ => {
                // Try to parse as UUID
                if let Ok(uuid) = uuid::Uuid::parse_str(&s) {
                    return Ok(Self::from(uuid.as_u128()));
                }
                Err(DataPathError::InvalidNameIdFormat(s))
            }
        }
    }
}

impl From<NameId> for String {
    fn from(name_id: NameId) -> String {
        let val: u128 = name_id.into();
        match val {
            NameId::NULL_COMPONENT => "NULL_COMPONENT".to_string(),
            NameId::DATA_CHANNEL_ID => "DATA_CHANNEL_ID".to_string(),
            NameId::CONTROL_CHANNEL_ID => "CONTROL_CHANNEL_ID".to_string(),
            id => uuid::Uuid::from_u128(id).to_string(),
        }
    }
}

impl EncodedName {
    /// Returns the u128 ID from the embedded `NameId`, or `NULL_COMPONENT` if absent.
    pub fn id(&self) -> u128 {
        self.name_id
            .as_ref()
            .map_or(NameId::NULL_COMPONENT, |nid| (*nid).into())
    }

    /// Returns the ID as a human-readable string.
    pub fn string_id(&self) -> String {
        self.name_id
            .as_ref()
            .map_or("NULL_COMPONENT".to_string(), |nid| (*nid).into())
    }
}

impl ProtoName {
    /// Construct from three string components (org, namespace, app).
    /// Encoded components are computed via XxHash64, matching the legacy `Name::from_strings` behaviour.
    pub fn from_strings(components: [impl Into<String>; 3]) -> Self {
        let [s0, s1, s2] = components.map(Into::into);
        Self {
            name: Some(EncodedName {
                component_0: calculate_hash(&s0),
                component_1: calculate_hash(&s1),
                component_2: calculate_hash(&s2),
                name_id: Some(NameId::from(NameId::NULL_COMPONENT)),
            }),
            str_name: Some(StringName {
                str_component_0: s0,
                str_component_1: s1,
                str_component_2: s2,
            }),
        }
    }

    /// Builder-style: set the ID (4th encoded component).
    pub fn with_id(mut self, id: u128) -> Self {
        self.name.as_mut().unwrap().name_id = Some(NameId::from(id));
        self
    }

    /// Returns the ID component
    pub fn id(&self) -> u128 {
        self.name.as_ref().unwrap().id()
    }

    pub fn string_id(&self) -> String {
        self.name.as_ref().unwrap().string_id()
    }

    pub fn name_id(&self) -> Option<NameId> {
        self.name.as_ref().unwrap().name_id
    }

    /// Returns `true` if an ID has been set (i.e. is not `NULL_COMPONENT`).
    pub fn has_id(&self) -> bool {
        self.id() != NameId::NULL_COMPONENT
    }

    /// Set the ID component in-place.
    pub fn set_id(&mut self, id: u128) {
        let id = NameId::from(id);
        self.name.as_mut().unwrap().name_id = Some(id);
    }

    /// Clear the ID component (reset to `NULL_COMPONENT`).
    pub fn reset_id(&mut self) {
        self.name.as_mut().unwrap().name_id = Some(NameId::from(NameId::NULL_COMPONENT));
    }

    /// Compare the first 3 encoded components (ignoring the ID).
    /// Used for subscription prefix matching.
    pub fn match_prefix(&self, other: &ProtoName) -> bool {
        let a = self.name.as_ref().unwrap();
        let b = other.name.as_ref().unwrap();
        a.component_0 == b.component_0
            && a.component_1 == b.component_1
            && a.component_2 == b.component_2
    }

    /// Returns the three human-readable string components as `(&str, &str, &str)`.
    pub fn str_components(&self) -> (&str, &str, &str) {
        let s = self.str_name.as_ref().unwrap();
        (&s.str_component_0, &s.str_component_1, &s.str_component_2)
    }

    /// Parse a name string in "org/namespace/app" format into a `ProtoName`.
    pub fn parse_name(s: &str) -> Result<ProtoName, DataPathError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(DataPathError::InvalidNameFormat(s.to_string()));
        }
        let parts: Vec<&str> = trimmed.split('/').map(str::trim).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            return Err(DataPathError::InvalidNameFormat(s.to_string()));
        }
        Ok(ProtoName::from_strings([parts[0], parts[1], parts[2]]))
    }
}

impl std::fmt::Display for ProtoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) = &self.str_name {
            write!(
                f,
                "{}/{}/{}/{}",
                s.str_component_0,
                s.str_component_1,
                s.str_component_2,
                self.name
                    .as_ref()
                    .and_then(|n| n.name_id)
                    .map_or("NULL_COMPONENT".to_string(), |id| id.to_string())
            )
        } else if let Some(enc) = &self.name {
            write!(
                f,
                "{}/{}/{}/{}",
                enc.component_0,
                enc.component_1,
                enc.component_2,
                enc.name_id
                    .as_ref()
                    .map_or("NULL_COMPONENT".to_string(), |id| id.to_string())
            )
        } else {
            write!(f, "<empty>")
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_proto_name_from_strings() {
        let n1 = ProtoName::from_strings(["Org", "Default", "App"]);
        let n2 = ProtoName::from_strings(["Org", "Default", "App"]);
        assert_eq!(n1, n2);
        let n3 = ProtoName::from_strings(["Other", "Default", "App"]);
        assert_ne!(n1, n3);
    }

    #[test]
    fn test_proto_name_with_id() {
        let n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        assert_eq!(n.id(), 42);
        assert!(n.has_id());
    }

    #[test]
    fn test_proto_name_reset_id() {
        let mut n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        n.reset_id();
        assert_eq!(n.id(), NameId::NULL_COMPONENT);
        assert!(!n.has_id());
    }

    #[test]
    fn test_proto_name_match_prefix() {
        let n1 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(1);
        let n2 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(999);
        assert!(n1.match_prefix(&n2));

        let n3 = ProtoName::from_strings(["Other", "Default", "App"]).with_id(1);
        assert!(!n1.match_prefix(&n3));
    }

    #[test]
    fn test_proto_name_str_components() {
        let n = ProtoName::from_strings(["org", "ns", "app"]);
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_proto_name_hash_stability() {
        // Verify that ProtoName::from_strings produces stable XxHash64 hashes
        // for known inputs.
        let proto = ProtoName::from_strings(["Org", "Default", "App"]).with_id(7);
        let enc = proto.name.unwrap();
        // Re-derive the same hash and confirm it matches
        let proto2 = ProtoName::from_strings(["Org", "Default", "App"]).with_id(7);
        let enc2 = proto2.name.unwrap();
        assert_eq!(enc, enc2);
        assert_eq!(enc.id(), 7);
    }

    #[test]
    fn test_name_id_roundtrip() {
        // Verify u128 -> NameId -> u128 roundtrip
        let values = [0u128, 1, 42, u128::MAX / 2, u128::MAX - 4];
        for v in values {
            let nid = NameId::from(v);
            let result: u128 = nid.into();
            assert_eq!(result, v, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_name_id_from_string_valid_uuid() {
        let nid = NameId::try_from("00000000-0000-0000-0000-00000000002a".to_string())
            .expect("valid UUID string should parse");
        let result: u128 = nid.into();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_name_id_from_string_invalid() {
        assert!(NameId::try_from("not-a-uuid".to_string()).is_err());
        assert!(NameId::try_from("".to_string()).is_err());
    }

    #[test]
    fn test_name_id_display_uuid() {
        let nid = NameId::from(42);
        assert_eq!(nid.to_string(), "00000000-0000-0000-0000-00000000002a");
    }

    #[test]
    fn test_name_id_display_reserved() {
        let mut str_nid: String = NameId::from(NameId::NULL_COMPONENT).into();
        assert_eq!(str_nid, "NULL_COMPONENT");
        str_nid = NameId::from(NameId::DATA_CHANNEL_ID).into();
        assert_eq!(str_nid, "DATA_CHANNEL_ID");
        str_nid = NameId::from(NameId::CONTROL_CHANNEL_ID).into();
        assert_eq!(str_nid, "CONTROL_CHANNEL_ID");
    }

    #[test]
    fn test_name_id_is_reserved() {
        assert!(NameId::is_reserved_id(NameId::NULL_COMPONENT));
        assert!(NameId::is_reserved_id(NameId::DATA_CHANNEL_ID));
        assert!(NameId::is_reserved_id(NameId::CONTROL_CHANNEL_ID));
        assert!(!NameId::is_reserved_id(0));
        assert!(!NameId::is_reserved_id(42));
    }

    #[test]
    fn test_proto_name_default_id_is_null_component() {
        let n = ProtoName::from_strings(["a", "b", "c"]);
        assert_eq!(n.id(), NameId::NULL_COMPONENT);
        assert!(!n.has_id());
    }

    #[test]
    fn test_proto_name_set_id() {
        let mut n = ProtoName::from_strings(["a", "b", "c"]);
        n.set_id(123);
        assert_eq!(n.id(), 123);
        assert!(n.has_id());
    }

    #[test]
    fn test_proto_name_string_id() {
        let n = ProtoName::from_strings(["a", "b", "c"]).with_id(42);
        assert_eq!(n.string_id(), "00000000-0000-0000-0000-00000000002a");
    }

    #[test]
    fn test_proto_name_display_with_id() {
        let n = ProtoName::from_strings(["org", "ns", "app"]).with_id(1);
        let s = format!("{}", n);
        assert_eq!(s, "org/ns/app/00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_proto_name_display_null_component() {
        let n = ProtoName::from_strings(["org", "ns", "app"]);
        let s = format!("{}", n);
        assert_eq!(s, "org/ns/app/NULL_COMPONENT");
    }

    #[test]
    fn test_encoded_name_id_when_name_id_none() {
        let enc = EncodedName {
            component_0: 0,
            component_1: 0,
            component_2: 0,
            name_id: None,
        };
        assert_eq!(enc.id(), NameId::NULL_COMPONENT);
        assert_eq!(enc.string_id(), "NULL_COMPONENT");
    }

    #[test]
    fn test_parse_name_valid() {
        let n = ProtoName::parse_name("org/ns/app").unwrap();
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_parse_name_trims_whitespace() {
        let n = ProtoName::parse_name(" org / ns / app ").unwrap();
        let (a, b, c) = n.str_components();
        assert_eq!(a, "org");
        assert_eq!(b, "ns");
        assert_eq!(c, "app");
    }

    #[test]
    fn test_parse_name_empty_string() {
        assert!(ProtoName::parse_name("").is_err());
        assert!(ProtoName::parse_name("   ").is_err());
    }

    #[test]
    fn test_parse_name_too_few_parts() {
        assert!(ProtoName::parse_name("org/ns").is_err());
        assert!(ProtoName::parse_name("org").is_err());
    }

    #[test]
    fn test_parse_name_too_many_parts() {
        assert!(ProtoName::parse_name("a/b/c/d").is_err());
    }

    #[test]
    fn test_parse_name_empty_component() {
        assert!(ProtoName::parse_name("org//app").is_err());
        assert!(ProtoName::parse_name("/ns/app").is_err());
        assert!(ProtoName::parse_name("org/ns/").is_err());
    }
}
