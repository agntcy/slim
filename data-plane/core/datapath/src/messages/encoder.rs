// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use twox_hash::XxHash64;

use crate::api::ProtoName;

#[derive(Clone)]
pub struct Name {
    /// The hashed components of the name (org, namespace, service)
    components: [u64; 3],

    /// The 128-bit identifier
    id: u128,

    // Store the original string representation of the components
    strings: Arc<[String; 3]>,
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.components[0].hash(state);
        self.components[1].hash(state);
        self.components[2].hash(state);
        self.id.hash(state);
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.components == other.components && self.id == other.id
    }
}

impl Eq for Name {}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}/{}", self.strings[0], self.strings[1], self.strings[2], format_id(self.id))
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id_str = format_id(self.id);
        write!(
            f,
            "{:x}/{:x}/{:x}/{} ({}/{}/{}/{})",
            self.components[0], self.components[1], self.components[2], id_str,
            self.strings[0], self.strings[1], self.strings[2], id_str,
        )
    }
}

impl From<&ProtoName> for Name {
    fn from(proto_name: &ProtoName) -> Self {
        let encoded = proto_name.name.as_ref().unwrap();
        let strings = proto_name.str_name.as_ref().unwrap();
        let id = id_from_bytes(&encoded.name_id);
        Self {
            components: [
                encoded.component_0,
                encoded.component_1,
                encoded.component_2,
            ],
            id,
            strings: Arc::new([
                strings.str_component_0.clone(),
                strings.str_component_1.clone(),
                strings.str_component_2.clone(),
            ]),
        }
    }
}

/// Convert a bytes slice to a u128 ID.
/// If the slice is exactly 16 bytes, interprets as big-endian u128.
/// If the slice is empty, returns NULL_COMPONENT.
/// Otherwise pads/truncates to 16 bytes (right-padded with 0xff for
/// backwards compatibility with shorter representations).
pub fn id_from_bytes(bytes: &[u8]) -> u128 {
    if bytes.is_empty() {
        return Name::NULL_COMPONENT;
    }
    if bytes.len() == 16 {
        return u128::from_be_bytes(bytes.try_into().unwrap());
    }
    // For shorter slices, right-pad with 0xff to preserve sentinel ordering
    let mut buf = [0xff_u8; 16];
    let len = bytes.len().min(16);
    buf[..len].copy_from_slice(&bytes[..len]);
    u128::from_be_bytes(buf)
}

/// Convert a u128 ID to a 16-byte big-endian representation.
pub fn id_to_bytes(id: u128) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

/// Format a 128-bit ID in a human-readable way.
/// Special sentinel values are printed by name; regular IDs use UUID-style hex.
pub fn format_id(id: u128) -> String {
    match id {
        Name::NULL_COMPONENT => "NULL_COMPONENT".to_string(),
        Name::DATA_CHANNEL_ID => "DATA_CHANNEL_ID".to_string(),
        Name::CONTROL_CHANNEL_ID => "CONTROL_CHANNEL_ID".to_string(),
        _ => {
            let bytes = id.to_be_bytes();
            format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                u32::from_be_bytes(bytes[0..4].try_into().unwrap()),
                u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
                u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
                u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
                u64::from_be_bytes({
                    let mut buf = [0u8; 8];
                    buf[2..8].copy_from_slice(&bytes[10..16]);
                    buf
                }),
            )
        }
    }
}

/// Parse a UUID-style hex string (or sentinel name) back into a u128 ID.
///
/// Accepted formats:
/// - `"NULL_COMPONENT"`, `"DATA_CHANNEL_ID"`, `"CONTROL_CHANNEL_ID"` (sentinel names)
/// - `"xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"` (UUID-style, 8-4-4-4-12 hex)
///
/// Returns `None` if the string doesn't match any accepted format.
pub fn parse_id(s: &str) -> Option<u128> {
    match s {
        "NULL_COMPONENT" => return Some(Name::NULL_COMPONENT),
        "DATA_CHANNEL_ID" => return Some(Name::DATA_CHANNEL_ID),
        "CONTROL_CHANNEL_ID" => return Some(Name::CONTROL_CHANNEL_ID),
        _ => {}
    }

    // Strip hyphens and parse as hex
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

impl Name {
    // NULL_COMPONENT is used to represent an id component that is not set
    pub const NULL_COMPONENT: u128 = u128::MAX;
    // Channels gets two different names: one for data and one for control
    // messages. The first 3 components are the same for both. Only the 4th
    // component (id) is different.
    // DATA_CHANNEL_ID is the id for the data channel name
    pub const DATA_CHANNEL_ID: u128 = u128::MAX - 2; // ends with 0xfd (data)
    // CONTROL_CHANNEL_ID is the id for the control channel name.
    pub const CONTROL_CHANNEL_ID: u128 = u128::MAX - 3; // ends with 0xfc (control))

    /// Returns true if `id` is one of the reserved values
    /// Notice that u128::MAX - 1 is not used at the moment
    /// and it reserved for future use.
    pub const fn is_reserved_id(id: u128) -> bool {
        id >= Self::CONTROL_CHANNEL_ID
    }

    pub fn from_strings(components: [impl Into<String>; 3]) -> Self {
        let strings = components.map(Into::into);

        Self {
            components: [
                calculate_hash(&strings[0]),
                calculate_hash(&strings[1]),
                calculate_hash(&strings[2]),
            ],
            id: Self::NULL_COMPONENT,
            strings: Arc::new(strings),
        }
    }

    pub fn with_id(self, id: u128) -> Self {
        Self {
            components: self.components,
            id,
            strings: self.strings,
        }
    }

    pub fn components(&self) -> &[u64; 3] {
        &self.components
    }

    pub fn id(&self) -> u128 {
        self.id
    }

    pub fn string_id(&self) -> String {
        format_id(self.id)
    }

    pub fn has_id(&self) -> bool {
        self.id != Self::NULL_COMPONENT
    }

    pub fn set_id(&mut self, id: u128) {
        self.id = id;
    }

    pub fn reset_id(&mut self) {
        self.id = Self::NULL_COMPONENT;
    }

    pub fn components_strings(&self) -> &[String; 3] {
        &self.strings
    }

    pub fn match_prefix(&self, other: &Name) -> bool {
        self.components == other.components
    }
}

pub fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut hasher = XxHash64::default();
    t.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_encoder() {
        let name1 = Name::from_strings(["Org", "Default", "App_ONE"]).with_id(1);
        let name2 = Name::from_strings(["Org", "Default", "App_ONE"]).with_id(1);
        assert_eq!(name1, name2);
        let name3 = Name::from_strings(["Another_Org", "Not_Default", "not_App_ONE"]).with_id(2);
        assert_ne!(name1, name3);
    }

    #[test]
    fn test_match_prefix() {
        // Test exact prefix match with same IDs
        let name1 = Name::from_strings(["Org", "Default", "App"]).with_id(1);
        let name2 = Name::from_strings(["Org", "Default", "App"]).with_id(1);
        assert!(name1.match_prefix(&name2));

        // Test exact prefix match with different IDs (should still match prefix)
        let name3 = Name::from_strings(["Org", "Default", "App"]).with_id(999);
        assert!(name1.match_prefix(&name3));

        // Test prefix match with no ID set
        let name4 = Name::from_strings(["Org", "Default", "App"]);
        assert!(name1.match_prefix(&name4));
        assert!(name4.match_prefix(&name1));

        // Test different first component
        let name5 = Name::from_strings(["DifferentOrg", "Default", "App"]).with_id(1);
        assert!(!name1.match_prefix(&name5));

        // Test different second component
        let name6 = Name::from_strings(["Org", "DifferentDefault", "App"]).with_id(1);
        assert!(!name1.match_prefix(&name6));

        // Test different third component
        let name7 = Name::from_strings(["Org", "Default", "DifferentApp"]).with_id(1);
        assert!(!name1.match_prefix(&name7));

        // Test completely different prefix
        let name8 = Name::from_strings(["NewOrg", "NewDefault", "NewApp"]).with_id(1);
        assert!(!name1.match_prefix(&name8));

        // Test self-match
        assert!(name1.match_prefix(&name1));
    }
}
