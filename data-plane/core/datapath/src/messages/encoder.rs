// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use bincode::{Decode, Encode};
use std::hash::{Hash, Hasher};
use twox_hash::XxHash64;

use crate::api::ProtoName;

#[derive(Clone, Encode, Decode)]
pub struct Name {
    /// The hashed components of the name
    components: [u64; 4],

    // Store the original string representation of the components
    strings: Box<[String; 3]>,
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.components[0].hash(state);
        self.components[1].hash(state);
        self.components[2].hash(state);
        self.components[3].hash(state);
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.components == other.components
    }
}

impl Eq for Name {}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}/{:x}",
            self.strings[0], self.strings[1], self.strings[2], self.components[3]
        )?;
        Ok(())
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:x}/{:x}/{:x}/{:x} ({}/{}/{}/{:x})",
            self.components[0],
            self.components[1],
            self.components[2],
            self.components[3],
            self.strings[0],
            self.strings[1],
            self.strings[2],
            self.components[3]
        )?;
        Ok(())
    }
}

impl From<&ProtoName> for Name {
    fn from(proto_name: &ProtoName) -> Self {
        let encoded = proto_name.name.unwrap();
        let strings = proto_name.str_name.as_ref().unwrap();
        Self {
            components: [
                encoded.component_0,
                encoded.component_1,
                encoded.component_2,
                encoded.component_3,
            ],
            strings: Box::new([
                strings.str_component_0.clone(),
                strings.str_component_1.clone(),
                strings.str_component_2.clone(),
            ]),
        }
    }
}

impl Name {
    // NULL_COMPONENT is used to represent a component that is not set
    pub const NULL_COMPONENT: u64 = u64::MAX;

    pub fn from_strings(components: [impl Into<String>; 3]) -> Self {
        let strings = components.map(Into::into);

        Self {
            components: [
                calculate_hash(&strings[0]),
                calculate_hash(&strings[1]),
                calculate_hash(&strings[2]),
                Self::NULL_COMPONENT,
            ],
            strings: Box::new(strings),
        }
    }

    pub fn with_id(self, id: u64) -> Self {
        Self {
            components: [
                self.components[0],
                self.components[1],
                self.components[2],
                id,
            ],
            strings: self.strings,
        }
    }

    pub fn components(&self) -> &[u64; 4] {
        &self.components
    }

    pub fn id(&self) -> u64 {
        self.components[3]
    }

    pub fn has_id(&self) -> bool {
        self.components[3] != Self::NULL_COMPONENT
    }

    pub fn set_id(&mut self, id: u64) {
        self.components[3] = id;
    }

    pub fn reset_id(&mut self) {
        self.components[3] = Self::NULL_COMPONENT;
    }

    pub fn components_strings(&self) -> &[String; 3] {
        &self.strings
    }

    pub fn match_prefix(&self, other: &Name) -> bool {
        self.components[0..3] == other.components[0..3]
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
