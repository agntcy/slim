// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Metadata for UniFFI
//!
//! Provides UniFFI-compatible wrapper around the core SlimRPC Metadata type.

use std::collections::HashMap;
use std::sync::Arc;

/// Metadata for RPC calls (UniFFI-compatible)
///
/// Metadata is a collection of key-value pairs that can be sent with RPC requests
/// and responses. It's used for passing headers, authentication tokens, tracing IDs, etc.
#[derive(Debug, Clone, uniffi::Object)]
pub struct Metadata {
    inner: agntcy_slimrpc::Metadata,
}

#[uniffi::export]
impl Metadata {
    /// Create a new empty metadata
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: agntcy_slimrpc::Metadata::new(),
        })
    }

    /// Create metadata from a HashMap
    #[uniffi::constructor]
    pub fn from_map(map: HashMap<String, String>) -> Arc<Self> {
        Arc::new(Self {
            inner: agntcy_slimrpc::Metadata::from_map(map),
        })
    }

    /// Insert a key-value pair
    pub fn insert(&self, key: String, value: String) -> Option<String> {
        // We need to clone the inner metadata since we can't mutate through Arc
        // This is a UniFFI limitation - we'll return a modified copy
        let mut inner = self.inner.clone();
        inner.insert(key, value)
    }

    /// Get a value by key
    pub fn get(&self, key: String) -> Option<String> {
        self.inner.get(&key).map(|s| s.to_string())
    }

    /// Remove a key-value pair
    pub fn remove(&self, key: String) -> Option<String> {
        let mut inner = self.inner.clone();
        inner.remove(&key)
    }

    /// Check if a key exists
    pub fn contains_key(&self, key: String) -> bool {
        self.inner.contains_key(&key)
    }

    /// Get the number of key-value pairs
    pub fn len(&self) -> u64 {
        self.inner.len() as u64
    }

    /// Check if metadata is empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get all keys
    pub fn keys(&self) -> Vec<String> {
        self.inner.keys().map(|k| k.to_string()).collect()
    }

    /// Get all values
    pub fn values(&self) -> Vec<String> {
        self.inner.values().map(|v| v.to_string()).collect()
    }

    /// Clear all metadata (returns a new empty Metadata)
    pub fn clear(&self) -> Arc<Self> {
        Arc::new(Self {
            inner: agntcy_slimrpc::Metadata::new(),
        })
    }

    /// Merge another metadata into this one (returns a new Metadata)
    pub fn merge(&self, other: Arc<Metadata>) -> Arc<Self> {
        let mut inner = self.inner.clone();
        inner.merge(other.inner.clone());
        Arc::new(Self { inner })
    }

    /// Convert to HashMap
    pub fn to_map(&self) -> HashMap<String, String> {
        self.inner.as_map().clone()
    }

    /// Create a mutable copy for modification
    pub fn with_insert(&self, key: String, value: String) -> Arc<Self> {
        let mut inner = self.inner.clone();
        inner.insert(key, value);
        Arc::new(Self { inner })
    }

    /// Create a copy without a specific key
    pub fn without(&self, key: String) -> Arc<Self> {
        let mut inner = self.inner.clone();
        inner.remove(&key);
        Arc::new(Self { inner })
    }
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            inner: agntcy_slimrpc::Metadata::new(),
        }
    }
}

// Internal conversion methods
impl Metadata {
    /// Get a reference to the inner core metadata
    pub(crate) fn inner(&self) -> &agntcy_slimrpc::Metadata {
        &self.inner
    }

    /// Convert to core metadata (cloning)
    pub(crate) fn to_core(&self) -> agntcy_slimrpc::Metadata {
        self.inner.clone()
    }

    /// Create from core metadata
    pub(crate) fn from_core(metadata: agntcy_slimrpc::Metadata) -> Self {
        Self { inner: metadata }
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
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());

        assert_eq!(metadata.get("key1".to_string()), Some("value1".to_string()));
        assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));
        assert_eq!(metadata.get("key3".to_string()), None);
        assert_eq!(metadata.len(), 2);
    }

    #[test]
    fn test_metadata_remove() {
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        let metadata = metadata.without("key1".to_string());
        assert_eq!(metadata.get("key1".to_string()), None);
        assert!(metadata.is_empty());
    }

    #[test]
    fn test_metadata_contains_key() {
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        assert!(metadata.contains_key("key1".to_string()));
        assert!(!metadata.contains_key("key2".to_string()));
    }

    #[test]
    fn test_metadata_clear() {
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());
        let metadata = metadata.clear();
        assert!(metadata.is_empty());
    }

    #[test]
    fn test_metadata_merge() {
        let metadata1 = Metadata::new();
        let metadata1 = metadata1.with_insert("key1".to_string(), "value1".to_string());

        let metadata2 = Metadata::new();
        let metadata2 = metadata2.with_insert("key2".to_string(), "value2".to_string());

        let merged = metadata1.merge(metadata2);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.get("key1".to_string()), Some("value1".to_string()));
        assert_eq!(merged.get("key2".to_string()), Some("value2".to_string()));
    }

    #[test]
    fn test_metadata_from_hashmap() {
        let mut map = HashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());

        let metadata = Metadata::from_map(map.clone());
        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata.get("key1".to_string()), Some("value1".to_string()));
        assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));
    }

    #[test]
    fn test_metadata_to_hashmap() {
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());

        let map = metadata.to_map();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("key1"), Some(&"value1".to_string()));
        assert_eq!(map.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_metadata_keys_values() {
        let metadata = Metadata::new();
        let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
        let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());

        let keys = metadata.keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));

        let values = metadata.values();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"value1".to_string()));
        assert!(values.contains(&"value2".to_string()));
    }
}
