// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC bindings for data plane service.
pub(crate) mod proto;

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
pub use proto::dataplane::v1::JoinReplyPayload;
pub use proto::dataplane::v1::JoinRequestPayload;
pub use proto::dataplane::v1::LeaveReplyPayload;
pub use proto::dataplane::v1::LeaveRequestPayload;
pub use proto::dataplane::v1::Link as ProtoLink;
pub use proto::dataplane::v1::LinkNegotiationPayload;
pub use proto::dataplane::v1::Message as ProtoMessage;
pub use proto::dataplane::v1::MlsPayload;
pub use proto::dataplane::v1::Name as ProtoName;
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

impl std::fmt::Display for ProtoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) = &self.str_name {
            write!(
                f,
                "{}/{}/{}",
                s.str_component_0, s.str_component_1, s.str_component_2
            )
        } else if let Some(enc) = &self.name {
            write!(
                f,
                "{}/{}/{}/{}",
                enc.component_0, enc.component_1, enc.component_2, enc.component_3
            )
        } else {
            write!(f, "<empty>")
        }
    }
}

impl ProtoName {
    /// Sentinel value indicating no ID is set (equivalent to `Name::NULL_COMPONENT`).
    pub const NULL_COMPONENT: u64 = u64::MAX;

    /// Returns `true` if `id` is the reserved null/unset sentinel.
    pub const fn is_reserved_id(id: u64) -> bool {
        id == Self::NULL_COMPONENT
    }

    /// Construct from three string components (org, namespace, app).
    /// Encoded components are computed via XxHash64, matching the legacy `Name::from_strings` behaviour.
    pub fn from_strings(components: [impl Into<String>; 3]) -> Self {
        let strings = components.map(Into::into);
        Self {
            name: Some(EncodedName {
                component_0: calculate_hash(&strings[0]),
                component_1: calculate_hash(&strings[1]),
                component_2: calculate_hash(&strings[2]),
                component_3: Self::NULL_COMPONENT,
            }),
            str_name: Some(StringName {
                str_component_0: strings[0].clone(),
                str_component_1: strings[1].clone(),
                str_component_2: strings[2].clone(),
            }),
        }
    }

    /// Builder-style: set the ID (4th encoded component).
    pub fn with_id(mut self, id: u64) -> Self {
        self.name.as_mut().unwrap().component_3 = id;
        self
    }

    /// Returns the ID component (4th encoded component), or `NULL_COMPONENT` if unset.
    pub fn id(&self) -> u64 {
        self.name
            .map(|e| e.component_3)
            .unwrap_or(Self::NULL_COMPONENT)
    }

    /// Returns `true` if an ID has been set (i.e. is not `NULL_COMPONENT`).
    pub fn has_id(&self) -> bool {
        self.id() != Self::NULL_COMPONENT
    }

    /// Set the ID component in-place.
    pub fn set_id(&mut self, id: u64) {
        self.name.as_mut().unwrap().component_3 = id;
    }

    /// Clear the ID component (reset to `NULL_COMPONENT`).
    pub fn reset_id(&mut self) {
        self.name.as_mut().unwrap().component_3 = Self::NULL_COMPONENT;
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
        assert_eq!(n.id(), ProtoName::NULL_COMPONENT);
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
    fn test_proto_name_matches_legacy_name_hashes() {
        // Verify that ProtoName::from_strings produces the same encoded components
        // as the legacy Name::from_strings (same XxHash64 algorithm).
        use crate::messages::encoder::Name;
        let legacy = Name::from_strings(["Org", "Default", "App"]).with_id(7);
        let proto = ProtoName::from_strings(["Org", "Default", "App"]).with_id(7);
        let enc = proto.name.unwrap();
        assert_eq!(enc.component_0, legacy.components()[0]);
        assert_eq!(enc.component_1, legacy.components()[1]);
        assert_eq!(enc.component_2, legacy.components()[2]);
        assert_eq!(enc.component_3, legacy.components()[3]);
    }
}
