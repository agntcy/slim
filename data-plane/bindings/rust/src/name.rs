// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Display;

use slim_datapath::messages::Name as SlimName;

use crate::errors::SlimError;

/// Name type for SLIM (Secure Low-Latency Interactive Messaging)
#[derive(Debug, Clone, PartialEq, uniffi::Object)]
#[uniffi::export(Display, Debug, Eq)]
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
    /// Create a new Name from components without an ID
    #[uniffi::constructor]
    pub fn new(component0: String, component1: String, component2: String) -> Self {
        let inner = SlimName::from_strings([component0, component1, component2]);
        Name { inner }
    }

    /// Parse a Name from a `"org/namespace/agent"` string
    ///
    /// The string must contain exactly three `/`-separated components.
    /// Returns an error if the format is invalid.
    #[uniffi::constructor]
    pub fn from_string(s: String) -> Result<Self, SlimError> {
        let parts: Vec<&str> = s.splitn(4, '/').collect();
        if parts.len() != 3 {
            return Err(SlimError::InvalidArgument {
                message: format!("expected \"org/namespace/agent\", got {:?}", s),
            });
        }
        Ok(Name {
            inner: SlimName::from_strings([parts[0], parts[1], parts[2]]),
        })
    }

    /// Create a new Name from components with an ID
    #[uniffi::constructor]
    pub fn new_with_id(
        component0: String,
        component1: String,
        component2: String,
        id: u64,
    ) -> Self {
        let inner = SlimName::from_strings([component0, component1, component2]).with_id(id);
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

    /// Convert to SlimName (for internal Rust use only, not exposed to FFI)
    pub fn as_slim_name(&self) -> SlimName {
        self.inner.clone()
    }

    /// Create from SlimName (for internal Rust use only, not exposed to FFI)
    pub fn from_slim_name(slim_name: SlimName) -> Self {
        Name { inner: slim_name }
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
        let name = Name::new_with_id(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            12345,
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
        let name = Name::new("org".to_string(), "".to_string(), "".to_string());

        let slim_name: SlimName = name.into();
        let components = slim_name.components_strings();

        assert_eq!(components[0], "org");
        assert_eq!(components[1], "");
        assert_eq!(components[2], "");
    }

    /// Test Name to SlimName conversion with empty components
    #[test]
    fn test_name_to_slim_name_empty() {
        let name = Name::new("".to_string(), "".to_string(), "".to_string());

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
        let original = Name::new_with_id(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            99999,
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
        let name1 = Name::new_with_id("a".to_string(), "b".to_string(), "c".to_string(), 100);
        let name2 = name1.clone();

        // PartialEq
        assert_eq!(name1, name2);

        // Different names should not be equal
        let name3 = Name::new_with_id("x".to_string(), "y".to_string(), "z".to_string(), 200);
        assert_ne!(name1, name3);

        // Debug
        let debug_str = format!("{:?}", name1);
        assert!(debug_str.contains("Name"));
        assert!(debug_str.contains("inner"));
    }

    /// Test Name Display trait
    #[test]
    fn test_name_display() {
        let name = Name::new_with_id(
            "org".to_string(),
            "namespace".to_string(),
            "app".to_string(),
            123,
        );
        let display_str = format!("{}", name);

        // Should display the SlimName format
        assert!(!display_str.is_empty());
    }

    /// Test Name with different ID values
    #[test]
    fn test_name_with_various_ids() {
        let name_with_id =
            Name::new_with_id("org".to_string(), "ns".to_string(), "app".to_string(), 42);
        assert_eq!(name_with_id.id(), 42);

        let name_without_id = Name::new("org".to_string(), "ns".to_string(), "app".to_string());
        // SlimName generates a default ID, so it should be non-zero
        assert!(name_without_id.id() > 0);
    }

    /// Test Name::from_string with a valid input
    #[test]
    fn test_from_string_valid() {
        let name = Name::from_string("org/namespace/agent".to_string()).unwrap();
        let components = name.components();
        assert_eq!(components[0], "org");
        assert_eq!(components[1], "namespace");
        assert_eq!(components[2], "agent");
    }

    /// Test Name::from_string roundtrips through Display
    #[test]
    fn test_from_string_roundtrip() {
        let original = Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "agent".to_string(),
        );
        let parsed = Name::from_string("org/namespace/agent".to_string()).unwrap();
        assert_eq!(original.components(), parsed.components());
    }

    /// Test Name::from_string rejects too few components
    #[test]
    fn test_from_string_too_few_components() {
        assert!(Name::from_string("org/namespace".to_string()).is_err());
        assert!(Name::from_string("org".to_string()).is_err());
        assert!(Name::from_string("".to_string()).is_err());
    }

    /// Test Name::from_string rejects too many components
    #[test]
    fn test_from_string_too_many_components() {
        assert!(Name::from_string("org/namespace/agent/extra".to_string()).is_err());
    }

    /// Test Name components getter
    #[test]
    fn test_name_components_getter() {
        let name = Name::new(
            "comp0".to_string(),
            "comp1".to_string(),
            "comp2".to_string(),
        );
        let components = name.components();

        assert_eq!(components.len(), 3);
        assert_eq!(components[0], "comp0");
        assert_eq!(components[1], "comp1");
        assert_eq!(components[2], "comp2");
    }
}
