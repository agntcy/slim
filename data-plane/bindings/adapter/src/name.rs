// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Display;

use slim_datapath::messages::Name as SlimName;

/// Name type for SLIM (Secure Low-Latency Interactive Messaging)
#[derive(Debug, Clone, PartialEq, uniffi::Object)]
pub struct Name {
    inner: SlimName,
}

impl From<Name> for SlimName {
    fn from(name: Name) -> Self {
        name.inner.clone()
    }
}

impl From<&Name> for SlimName {
    fn from(name: &Name) -> Self {
        name.inner.clone()
    }
}

impl From<SlimName> for Name {
    fn from(name: SlimName) -> Self {
        Name { inner: name }
    }
}

impl From<&SlimName> for Name {
    fn from(name: &SlimName) -> Self {
        Name {
            inner: name.clone(),
        }
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

#[uniffi::export]
impl Name {
    /// Create a new Name from a string
    #[uniffi::constructor]
    pub fn new(
        component_0: String,
        component1: String,
        component2: String,
        id: Option<u64>,
    ) -> Self {
        let mut inner = SlimName::from_strings([component_0, component1, component2]);
        if let Some(id_value) = id {
            inner = inner.with_id(id_value);
        }
        Name { inner }
    }

    /// Get the name components as a vector of strings
    pub fn components(&self) -> Vec<String> {
        self.inner.components_strings().to_vec()
    }

    /// Get the name ID
    pub fn id(&self) -> u64 {
        self.inner.id()
    }
}

impl Name {
    /// Get the name components as a reference (for internal Rust use only, not exposed to FFI)
    ///
    /// This avoids copying when used internally in Rust code.
    pub fn components_ref(&self) -> &[String; 3] {
        self.inner.components_strings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Name Conversion Tests
    // ========================================================================

    /// Test Name to SlimName conversion with full components
    #[test]
    fn test_name_to_slim_name_full() {
        let name = Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            Some(12345),
        );

        let slim_name: SlimName = name.into();
        let components = slim_name.components_strings();

        assert_eq!(components[0], "org");
        assert_eq!(components[1], "namespace");
        assert_eq!(components[2], "app");
        assert_eq!(slim_name.id(), 12345);
    }

    /// Test Name to SlimName conversion with partial components
    #[test]
    fn test_name_to_slim_name_partial() {
        let name = Name::new("org".to_string(), "".to_string(), "".to_string(), None);

        let slim_name: SlimName = name.into();
        let components = slim_name.components_strings();

        assert_eq!(components[0], "org");
        assert_eq!(components[1], "");
        assert_eq!(components[2], "");
    }

    /// Test Name to SlimName conversion with empty components
    #[test]
    fn test_name_to_slim_name_empty() {
        let name = Name::new("".to_string(), "".to_string(), "".to_string(), None);

        let slim_name: SlimName = name.into();
        let components = slim_name.components_strings();

        assert_eq!(components[0], "");
        assert_eq!(components[1], "");
        assert_eq!(components[2], "");
    }

    /// Test SlimName to Name conversion
    #[test]
    fn test_slim_name_to_name() {
        let slim_name = SlimName::from_strings(["org", "namespace", "app"]).with_id(54321);

        let name = Name::from(&slim_name);

        assert_eq!(name.components(), vec!["org", "namespace", "app"]);
        assert_eq!(name.id(), 54321);
    }

    /// Test Name roundtrip conversion
    #[test]
    fn test_name_roundtrip() {
        let original = Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            Some(99999),
        );

        let slim_name: SlimName = original.clone().into();
        let converted = Name::from(&slim_name);

        assert_eq!(original.components(), converted.components());
        assert_eq!(original.id(), converted.id());
    }

    // ========================================================================
    // Name Traits Tests
    // ========================================================================

    /// Test Name Debug, Clone, and PartialEq traits
    #[test]
    fn test_name_traits() {
        let name1 = Name::new("a".to_string(), "b".to_string(), "c".to_string(), Some(100));
        let name2 = name1.clone();

        // PartialEq
        assert_eq!(name1, name2);

        // Different names should not be equal
        let name3 = Name::new("x".to_string(), "y".to_string(), "z".to_string(), Some(200));
        assert_ne!(name1, name3);

        // Debug
        let debug_str = format!("{:?}", name1);
        assert!(debug_str.contains("Name"));
        assert!(debug_str.contains("inner"));
    }

    /// Test Name Display trait
    #[test]
    fn test_name_display() {
        let name = Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            Some(123),
        );
        let display_str = format!("{}", name);

        // Should display the SlimName format
        assert!(!display_str.is_empty());
    }

    /// Test Name with different ID values
    #[test]
    fn test_name_with_various_ids() {
        let name_with_id = Name::new(
            "org".to_string(),
            "ns".to_string(),
            "app".to_string(),
            Some(42),
        );
        assert_eq!(name_with_id.id(), 42);

        let name_without_id =
            Name::new("org".to_string(), "ns".to_string(), "app".to_string(), None);
        // SlimName generates a default ID, so it should be non-zero
        assert!(name_without_id.id() > 0);
    }

    /// Test Name components getter
    #[test]
    fn test_name_components_getter() {
        let name = Name::new(
            "comp0".to_string(),
            "comp1".to_string(),
            "comp2".to_string(),
            None,
        );
        let components = name.components();

        assert_eq!(components.len(), 3);
        assert_eq!(components[0], "comp0");
        assert_eq!(components[1], "comp1");
        assert_eq!(components[2], "comp2");
    }
}
