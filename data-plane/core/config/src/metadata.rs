// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Generic, arbitrarily nested metadata map structure that can be attached to
//! configuration structs. Each value can be a string, number, list or another map.
//! This provides an escape hatch for custom configuration without changing the
//! strongly typed configuration surface.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A generic metadata value.
///
/// Supported variants:
/// - String
/// - Number (serde_json::Number – can represent integer & floating point)
/// - List (Vec<MetadataValue>)
/// - Map (nested MetadataMap)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MetadataValue {
    String(String),
    Number(serde_json::Number),
    List(Vec<MetadataValue>),
    Map(MetadataMap),
}

impl From<String> for MetadataValue {
    fn from(value: String) -> Self {
        MetadataValue::String(value)
    }
}

impl From<&str> for MetadataValue {
    fn from(value: &str) -> Self {
        MetadataValue::String(value.to_string())
    }
}

impl From<i64> for MetadataValue {
    fn from(value: i64) -> Self {
        MetadataValue::Number(serde_json::Number::from(value))
    }
}

impl From<u64> for MetadataValue {
    fn from(value: u64) -> Self {
        MetadataValue::Number(serde_json::Number::from(value))
    }
}

impl From<f64> for MetadataValue {
    fn from(value: f64) -> Self {
        serde_json::Number::from_f64(value)
            .map(MetadataValue::Number)
            .unwrap_or_else(|| MetadataValue::String(value.to_string()))
    }
}

impl<T: Into<MetadataValue>> From<Vec<T>> for MetadataValue {
    fn from(v: Vec<T>) -> Self {
        MetadataValue::List(v.into_iter().map(|e| e.into()).collect())
    }
}

impl From<MetadataMap> for MetadataValue {
    fn from(m: MetadataMap) -> Self {
        MetadataValue::Map(m)
    }
}

/// A generic metadata map. Newtype with a flattened map so that serde encodes
/// just a JSON/YAML object and not an inner field name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, JsonSchema)]
pub struct MetadataMap {
    #[serde(flatten)]
    pub inner: HashMap<String, MetadataValue>,
}

impl MetadataMap {
    /// Create an empty metadata map.
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Insert any value implementing Into<MetadataValue>.
    pub fn insert<K: Into<String>, V: Into<MetadataValue>>(&mut self, key: K, value: V) {
        self.inner.insert(key.into(), value.into());
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&MetadataValue> {
        self.inner.get(key)
    }

    /// Mutable get.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut MetadataValue> {
        self.inner.get_mut(key)
    }

    /// Returns true if map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Length of the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn insert_and_get_primitives() {
        let mut m = MetadataMap::new();
        m.insert("s", "hello");
        m.insert("i", 42i64);
        m.insert("u", 7u64);
        m.insert("f", std::f64::consts::PI);

        assert!(matches!(m.get("s"), Some(MetadataValue::String(v)) if v == "hello"));
        assert!(matches!(m.get("i"), Some(MetadataValue::Number(n)) if n.as_i64()==Some(42)));
        assert!(matches!(m.get("u"), Some(MetadataValue::Number(n)) if n.as_u64()==Some(7)));
        // Float may become string fallback if NaN; here it's valid
        assert!(matches!(m.get("f"), Some(MetadataValue::Number(_))));
    }

    #[test]
    fn list_and_nested_map() {
        let mut child = MetadataMap::new();
        child.insert("k", "v");

        let mut root = MetadataMap::new();
        root.insert("list", vec![1i64, 2i64, 3i64]);
        root.insert("child", child.clone());

        match root.get("list").unwrap() {
            MetadataValue::List(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected list"),
        }
        match root.get("child").unwrap() {
            MetadataValue::Map(m) => {
                assert!(matches!(m.get("k"), Some(MetadataValue::String(s)) if s == "v"))
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn serialize_to_json() {
        let mut m = MetadataMap::new();
        m.insert("name", "slim");
        m.insert("version", 1u64);
        m.insert("values", vec!["a", "b", "c"]);
        let mut nested = MetadataMap::new();
        nested.insert("inner", 10i64);
        m.insert("nested", nested);

        let json_str = serde_json::to_string(&m).unwrap();
        let v: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["name"], json!("slim"));
        assert_eq!(v["version"], json!(1));
        assert_eq!(v["values"], json!(["a", "b", "c"]));
        assert_eq!(v["nested"]["inner"], json!(10));
    }

    #[test]
    fn deserialize_from_json() {
        let raw = r#"{
            "alpha":"a",
            "num": 5,
            "list": [1,2,3],
            "deep": {"k":"v","n":9}
        }"#;
        let m: MetadataMap = serde_json::from_str(raw).unwrap();
        assert!(matches!(m.get("alpha"), Some(MetadataValue::String(s)) if s=="a"));
        assert!(matches!(m.get("num"), Some(MetadataValue::Number(n)) if n.as_i64()==Some(5)));
        assert!(matches!(m.get("list"), Some(MetadataValue::List(v)) if v.len()==3));
        match m.get("deep").unwrap() {
            MetadataValue::Map(dm) => {
                assert!(matches!(dm.get("k"), Some(MetadataValue::String(s)) if s=="v"));
                assert!(
                    matches!(dm.get("n"), Some(MetadataValue::Number(n)) if n.as_i64()==Some(9))
                );
            }
            _ => panic!("expected deep map"),
        }
    }

    #[test]
    fn overwrite_key() {
        let mut m = MetadataMap::new();
        m.insert("k", 1i64);
        m.insert("k", "now_string");
        assert!(matches!(m.get("k"), Some(MetadataValue::String(s)) if s=="now_string"));
    }

    #[test]
    fn schema_generation() {
        // Ensure schemars can generate a schema (smoke test)
        let _schema = schemars::schema_for!(MetadataMap);
    }

    #[test]
    fn serialize_to_yaml() {
        let mut m = MetadataMap::new();
        m.insert("service", "edge");
        m.insert("version", 2u64);
        m.insert("values", vec!["x", "y"]);
        let mut nested = MetadataMap::new();
        nested.insert("enabled", "true"); // store as string intentionally
        nested.insert("count", 5u64);
        m.insert("nested", nested.clone());

        let yaml_str = serde_yaml::to_string(&m).unwrap();
        // round-trip
        let back: MetadataMap = serde_yaml::from_str(&yaml_str).unwrap();
        assert_eq!(back.get("service"), m.get("service"));
        assert_eq!(back.get("version"), m.get("version"));
        assert_eq!(back.get("values"), m.get("values"));
        assert_eq!(back.get("nested"), m.get("nested"));

        // Ensure nested structure preserved
        if let Some(MetadataValue::Map(n)) = back.get("nested") {
            assert!(
                matches!(n.get("count"), Some(MetadataValue::Number(num)) if num.as_u64()==Some(5))
            );
        } else {
            panic!("missing nested map");
        }
    }

    #[test]
    fn deserialize_from_yaml() {
        let raw = r#"
service: api
nums: [1, 2, 3]
meta:
  key: value
  num: 9
"#;
        let m: MetadataMap = serde_yaml::from_str(raw).unwrap();
        assert!(matches!(m.get("service"), Some(MetadataValue::String(s)) if s=="api"));
        assert!(matches!(m.get("nums"), Some(MetadataValue::List(v)) if v.len()==3));
        match m.get("meta").unwrap() {
            MetadataValue::Map(mm) => {
                assert!(matches!(mm.get("key"), Some(MetadataValue::String(s)) if s=="value"));
                assert!(
                    matches!(mm.get("num"), Some(MetadataValue::Number(n)) if n.as_i64()==Some(9))
                );
            }
            _ => panic!("expected map"),
        }
    }
}
