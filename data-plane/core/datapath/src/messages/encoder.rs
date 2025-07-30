// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use bincode::{Decode, Encode};
use std::hash::{Hash, Hasher};
use twox_hash::XxHash64;

use crate::api::ProtoName;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Name {
    /// The hashed components of the name
    components: [u64; 4],

    // Store the original string representation of the components
    // This is useful for debugging and logging purposes
    strings: Option<Box<[String; 3]>>,
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
            "{:x}/{:x}/{:x}/{:x}",
            self.components[0], self.components[1], self.components[2], self.components[3]
        )?;

        if let Some(strings) = &self.strings {
            write!(
                f,
                " ({}/{}/{}/{:x})",
                strings[0], strings[1], strings[2], self.components[3]
            )?;
        }

        Ok(())
    }
}

impl From<&ProtoName> for Name {
    fn from(proto_name: &ProtoName) -> Self {
        Self {
            components: [
                proto_name.component_0,
                proto_name.component_1,
                proto_name.component_2,
                proto_name.component_3,
            ],
            strings: None,
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
            strings: Some(Box::new(strings)),
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

    pub fn set_id(&mut self, id: u64) {
        self.components[3] = id;
    }

    pub fn reset_id(&mut self) {
        self.components[3] = Self::NULL_COMPONENT;
    }

    pub fn components_strings(&self) -> Option<&[String; 3]> {
        self.strings.as_deref()
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
}
