// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Metadata handling for SlimRPC
//!
//! Provides a type-safe wrapper around metadata (key-value pairs) passed with RPC calls.

use std::collections::HashMap;

/// Metadata for RPC calls
///
/// Metadata is a collection of key-value pairs that can be sent with RPC requests
/// and responses. It's used for passing headers, authentication tokens, tracing IDs, etc.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct Metadata {
    inner: HashMap<String, String>,
}

impl Metadata {
    /// Create a new empty metadata
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Create metadata from a HashMap
    pub fn from_map(map: HashMap<String, String>) -> Self {
        Self { inner: map }
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> Option<String> {
        self.inner.insert(key.into(), value.into())
    }

    /// Get a value by key
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key).map(|s| s.as_str())
    }

    /// Remove a key-value pair
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.inner.remove(key)
    }

    /// Check if a key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Get the number of key-value pairs
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if metadata is empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over key-value pairs
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Get all keys
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|k| k.as_str())
    }

    /// Get all values
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.inner.values().map(|v| v.as_str())
    }

    /// Clear all metadata
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Merge another metadata into this one
    pub fn merge(&mut self, other: Metadata) {
        self.inner.extend(other.inner);
    }

    /// Convert to HashMap
    pub fn into_map(self) -> HashMap<String, String> {
        self.inner
    }

    /// Get a reference to the inner HashMap
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.inner
    }

    /// Get a mutable reference to the inner HashMap
    pub fn as_map_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.inner
    }
}

impl From<HashMap<String, String>> for Metadata {
    fn from(map: HashMap<String, String>) -> Self {
        Self::from_map(map)
    }
}

impl From<Metadata> for HashMap<String, String> {
    fn from(metadata: Metadata) -> Self {
        metadata.inner
    }
}

impl FromIterator<(String, String)> for Metadata {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        Self {
            inner: HashMap::from_iter(iter),
        }
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for Metadata {
    fn from_iter<T: IntoIterator<Item = (&'a str, &'a str)>>(iter: T) -> Self {
        Self {
            inner: iter
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

impl Extend<(String, String)> for Metadata {
    fn extend<T: IntoIterator<Item = (String, String)>>(&mut self, iter: T) {
        self.inner.extend(iter);
    }
}

impl<'a> Extend<(&'a str, &'a str)> for Metadata {
    fn extend<T: IntoIterator<Item = (&'a str, &'a str)>>(&mut self, iter: T) {
        self.inner.extend(
            iter.into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_new() {
        let metadata = Metadata::new();
        assert!(metadata.is_empty());
        assert_eq!(metadata.len(), 0);
    }

    #[test]
    fn test_metadata_insert_get() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        metadata.insert("key2", "value2");

        assert_eq!(metadata.get("key1"), Some("value1"));
        assert_eq!(metadata.get("key2"), Some("value2"));
        assert_eq!(metadata.get("key3"), None);
        assert_eq!(metadata.len(), 2);
    }

    #[test]
    fn test_metadata_remove() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        assert_eq!(metadata.remove("key1"), Some("value1".to_string()));
        assert_eq!(metadata.get("key1"), None);
        assert!(metadata.is_empty());
    }

    #[test]
    fn test_metadata_contains_key() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        assert!(metadata.contains_key("key1"));
        assert!(!metadata.contains_key("key2"));
    }

    #[test]
    fn test_metadata_clear() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        metadata.insert("key2", "value2");
        metadata.clear();
        assert!(metadata.is_empty());
    }

    #[test]
    fn test_metadata_merge() {
        let mut metadata1 = Metadata::new();
        metadata1.insert("key1", "value1");

        let mut metadata2 = Metadata::new();
        metadata2.insert("key2", "value2");

        metadata1.merge(metadata2);
        assert_eq!(metadata1.len(), 2);
        assert_eq!(metadata1.get("key1"), Some("value1"));
        assert_eq!(metadata1.get("key2"), Some("value2"));
    }

    #[test]
    fn test_metadata_from_hashmap() {
        let mut map = HashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());

        let metadata = Metadata::from_map(map.clone());
        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata.get("key1"), Some("value1"));
        assert_eq!(metadata.get("key2"), Some("value2"));
    }

    #[test]
    fn test_metadata_into_hashmap() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        metadata.insert("key2", "value2");

        let map: HashMap<String, String> = metadata.into_map();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("key1"), Some(&"value1".to_string()));
        assert_eq!(map.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_metadata_iter() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");
        metadata.insert("key2", "value2");

        let items: Vec<_> = metadata.iter().collect();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_metadata_from_iter() {
        let items = vec![("key1", "value1"), ("key2", "value2")];
        let metadata: Metadata = items.into_iter().collect();

        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata.get("key1"), Some("value1"));
        assert_eq!(metadata.get("key2"), Some("value2"));
    }

    #[test]
    fn test_metadata_extend() {
        let mut metadata = Metadata::new();
        metadata.insert("key1", "value1");

        metadata.extend(vec![("key2", "value2"), ("key3", "value3")]);
        assert_eq!(metadata.len(), 3);
        assert_eq!(metadata.get("key2"), Some("value2"));
        assert_eq!(metadata.get("key3"), Some("value3"));
    }
}
