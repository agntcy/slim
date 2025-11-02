// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::{self, Deserialize, Deserializer, Visitor};
use std::fmt;
use std::ops;

// Define the OpaqueString struct
#[derive(Clone, PartialEq, Serialize, JsonSchema)]
pub struct OpaqueString(String);

impl OpaqueString {
    // Constructor to create a new OpaqueString
    pub fn new(value: &str) -> Self {
        OpaqueString(value.to_string())
    }
}

// Implement the Display trait to print asterisks
impl fmt::Display for OpaqueString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", "*".repeat(self.0.len()))
    }
}

// Implement the Debug trait to also print asterisks
impl fmt::Debug for OpaqueString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", "*".repeat(self.0.len()))
    }
}

// Implement Deref and DerefMut to allow transparent access to String methods
impl ops::Deref for OpaqueString {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for OpaqueString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// Optionally, implement From<String> and From<&str> for convenience
impl From<String> for OpaqueString {
    fn from(value: String) -> Self {
        OpaqueString(value)
    }
}

impl From<&str> for OpaqueString {
    fn from(value: &str) -> Self {
        OpaqueString(value.to_string())
    }
}

// implement PartialEq to compare OpaqueString with String
impl PartialEq<String> for OpaqueString {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

// Also implement a deserializer to be used with serde
impl<'de> Deserialize<'de> for OpaqueString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OpaqueStringVisitor;

        impl Visitor<'_> for OpaqueStringVisitor {
            type Value = OpaqueString;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OpaqueString::new(value))
            }
        }

        deserializer.deserialize_str(OpaqueStringVisitor)
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opaque_string() {
        let value = "password";
        let opaque = OpaqueString::new(value);

        assert_eq!(format!("{}", opaque), "********");
        assert_eq!(format!("{:?}", opaque), "********");
        assert_eq!(opaque.len(), value.len());
        assert!(!opaque.is_empty());
        assert!(opaque.contains("pass"));
        assert!(opaque.contains("word"));
        assert!(opaque.contains("passw"));
        assert!(!opaque.contains("wordd")); // spellchecker:disable-line
        assert!(opaque.starts_with("pass"));
        assert!(opaque.ends_with("word"));
        assert!(!opaque.ends_with("worrd"));
        assert_eq!(opaque.to_uppercase(), "PASSWORD");
        assert_eq!(opaque.to_lowercase(), "password");
        assert_eq!(opaque.trim(), "password");
        assert_eq!(opaque.trim_start(), "password");
        assert_eq!(opaque.trim_end(), "password");
        assert_eq!(opaque.trim_matches(|c: char| c == 'd'), "passwor"); // spellchecker:disable-line
    }

    #[test]
    fn test_opaque_string_deserialize() {
        let value = "password";
        let opaque = OpaqueString::new(value);

        let deserialized: OpaqueString = serde_json::from_str(&format!("\"{}\"", value)).unwrap();
        assert_eq!(deserialized, opaque);
    }
}
