// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Generic, arbitrarily nested metadata map structure that can be attached to
//! configuration structs. Each value can be a string, number, list or another map.
//! This provides an escape hatch for custom configuration without changing the
//! strongly typed configuration surface.

use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::fmt;

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

impl MetadataValue {
    /// Return the variant name (for error reporting).
    fn variant_name(&self) -> &'static str {
        match self {
            MetadataValue::String(_) => "String",
            MetadataValue::Number(_) => "Number",
            MetadataValue::List(_) => "List",
            MetadataValue::Map(_) => "Map",
        }
    }

    /// Convenience accessors.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            MetadataValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<&serde_json::Number> {
        match self {
            MetadataValue::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[MetadataValue]> {
        match self {
            MetadataValue::List(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&MetadataMap> {
        match self {
            MetadataValue::Map(m) => Some(m),
            _ => None,
        }
    }
}

/// Error type returned when attempting to convert (try_into) a `MetadataValue`
/// into a concrete type and the value does not have the expected shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataConversionError {
    WrongType {
        expected: &'static str,
        found: &'static str,
    },
    NumberOutOfRange {
        expected: &'static str,
        found: serde_json::Number,
    },
    /// Returned when a floating point number cannot be represented (unlikely here,
    /// but preserved for completeness).
    FloatNotRepresentable,
}

impl fmt::Display for MetadataConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetadataConversionError::WrongType { expected, found } => {
                write!(
                    f,
                    "wrong metadata value type: expected {expected}, found {found}"
                )
            }
            MetadataConversionError::NumberOutOfRange { expected, found } => {
                write!(
                    f,
                    "number out of range when converting metadata value: expected {expected}, found {found}"
                )
            }
            MetadataConversionError::FloatNotRepresentable => {
                write!(f, "float not representable for metadata value")
            }
        }
    }
}

impl Error for MetadataConversionError {}

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
        // serde_json::Number cannot represent NaN/Infinity; fall back to string.
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

/* -------- TryFrom implementations for extraction -------- */

impl TryFrom<MetadataValue> for String {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::String(s) => Ok(s),
            other => Err(MetadataConversionError::WrongType {
                expected: "String",
                found: other.variant_name(),
            }),
        }
    }
}

impl<'a> TryFrom<&'a MetadataValue> for &'a str {
    type Error = MetadataConversionError;

    fn try_from(value: &'a MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::String(s) => Ok(s.as_str()),
            other => Err(MetadataConversionError::WrongType {
                expected: "String",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for String {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::String(s) => Ok(s.clone()),
            other => Err(MetadataConversionError::WrongType {
                expected: "String",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for serde_json::Number {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => Ok(n),
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for serde_json::Number {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => Ok(n.clone()),
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for i64 {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => {
                n.as_i64().ok_or(MetadataConversionError::NumberOutOfRange {
                    expected: "i64",
                    found: n,
                })
            }
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for i64 {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => {
                n.as_i64().ok_or(MetadataConversionError::NumberOutOfRange {
                    expected: "i64",
                    found: n.clone(),
                })
            }
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for u64 {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => {
                n.as_u64().ok_or(MetadataConversionError::NumberOutOfRange {
                    expected: "u64",
                    found: n,
                })
            }
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for u64 {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => {
                n.as_u64()
                    .ok_or_else(|| MetadataConversionError::NumberOutOfRange {
                        expected: "u64",
                        found: n.clone(),
                    })
            }
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for f64 {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => n
                .as_f64()
                .ok_or(MetadataConversionError::FloatNotRepresentable),
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for f64 {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Number(n) => n
                .as_f64()
                .ok_or(MetadataConversionError::FloatNotRepresentable),
            other => Err(MetadataConversionError::WrongType {
                expected: "Number",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for Vec<MetadataValue> {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::List(v) => Ok(v),
            other => Err(MetadataConversionError::WrongType {
                expected: "List",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for Vec<MetadataValue> {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::List(v) => Ok(v.clone()),
            other => Err(MetadataConversionError::WrongType {
                expected: "List",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<MetadataValue> for MetadataMap {
    type Error = MetadataConversionError;

    fn try_from(value: MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Map(m) => Ok(m),
            other => Err(MetadataConversionError::WrongType {
                expected: "Map",
                found: other.variant_name(),
            }),
        }
    }
}

impl TryFrom<&MetadataValue> for MetadataMap {
    type Error = MetadataConversionError;

    fn try_from(value: &MetadataValue) -> Result<Self, Self::Error> {
        match value {
            MetadataValue::Map(m) => Ok(m.clone()),
            other => Err(MetadataConversionError::WrongType {
                expected: "Map",
                found: other.variant_name(),
            }),
        }
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

    /// Iterate over key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &MetadataValue)> {
        self.inner.iter()
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

    /// Extend metadata map with another.
    pub fn extend(&mut self, other: MetadataMap) {
        self.inner.extend(other.inner);
    }
}

impl<K, V> std::iter::FromIterator<(K, V)> for MetadataMap
where
    K: Into<String>,
    V: Into<MetadataValue>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = MetadataMap::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use std::f64;

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
    fn collect_into_metadata_map() {
        let pairs = vec![("a", "alpha"), ("b", "beta"), ("c", "gamma")];
        let m: MetadataMap = pairs.into_iter().collect();
        assert_eq!(m.get("a").unwrap().as_str(), Some("alpha"));
        assert_eq!(m.get("b").unwrap().as_str(), Some("beta"));
        assert_eq!(m.get("c").unwrap().as_str(), Some("gamma"));
    }

    #[test]
    fn try_from_string_ok() {
        let v: MetadataValue = "hello".into();
        let s: String = v.clone().try_into().unwrap();
        assert_eq!(s, "hello");
        let s_ref: &str = (&v).try_into().unwrap();
        assert_eq!(s_ref, "hello");
    }

    #[test]
    fn try_from_number_variants() {
        let vu: MetadataValue = 10u64.into();
        assert_eq!(u64::try_from(vu.clone()).unwrap(), 10);
        assert_eq!(i64::try_from(vu.clone()).unwrap(), 10);
        let vf: MetadataValue = f64::consts::PI.into();
        let f = f64::try_from(vf.clone()).unwrap();
        assert!((f - f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn try_from_wrong_type_errors() {
        let v: MetadataValue = vec![1i64, 2i64].into();
        let err = String::try_from(v.clone()).unwrap_err();
        match err {
            MetadataConversionError::WrongType { expected, found } => {
                assert_eq!(expected, "String");
                assert_eq!(found, "List");
            }
            _ => panic!("unexpected error variant"),
        }
        let err_num = i64::try_from(v.clone()).unwrap_err();
        match err_num {
            MetadataConversionError::WrongType { expected, found } => {
                assert_eq!(expected, "Number");
                assert_eq!(found, "List");
            }
            _ => panic!("unexpected error variant"),
        }
    }

    #[test]
    fn number_out_of_range_for_i64() {
        let big = MetadataValue::from(u64::MAX);
        let err = i64::try_from(big).unwrap_err();
        match err {
            MetadataConversionError::NumberOutOfRange { expected, .. } => {
                assert_eq!(expected, "i64");
            }
            _ => panic!("expected NumberOutOfRange"),
        }
    }

    #[test]
    fn extract_list_and_map() {
        let mut child = MetadataMap::new();
        child.insert("x", 1i64);

        let list_val: MetadataValue = vec![1i64, 2i64].into();
        let extracted_list: Vec<MetadataValue> = list_val.clone().try_into().unwrap();
        assert_eq!(extracted_list.len(), 2);

        let map_val: MetadataValue = child.clone().into();
        let extracted_map: MetadataMap = map_val.try_into().unwrap();
        assert_eq!(
            extracted_map
                .get("x")
                .unwrap()
                .as_number()
                .unwrap()
                .as_i64(),
            Some(1)
        );
    }

    #[test]
    fn accessors_work() {
        let s: MetadataValue = "abc".into();
        assert_eq!(s.as_str(), Some("abc"));
        assert!(s.as_number().is_none());

        let n: MetadataValue = 42i64.into();
        assert_eq!(n.as_number().unwrap().as_i64(), Some(42));
        assert!(n.as_list().is_none());

        let lst: MetadataValue = vec![1i64, 2i64].into();
        assert_eq!(lst.as_list().unwrap().len(), 2);
        assert!(lst.as_map().is_none());

        let mut m = MetadataMap::new();
        m.insert("k", "v");
        let mv: MetadataValue = m.clone().into();
        assert_eq!(mv.as_map().unwrap().get("k").unwrap().as_str(), Some("v"));
    }

    #[test]
    fn try_from_borrowed_refs() {
        let v_num: MetadataValue = 7u64.into();
        let borrowed_u: u64 = (&v_num).try_into().unwrap();
        assert_eq!(borrowed_u, 7);
        let borrowed_i: i64 = (&v_num).try_into().unwrap();
        assert_eq!(borrowed_i, 7);

        let v_str: MetadataValue = "hello".into();
        let borrowed_str: &str = (&v_str).try_into().unwrap();
        assert_eq!(borrowed_str, "hello");

        let v_list: MetadataValue = vec!["a", "b"].into();
        let borrowed_list: Vec<MetadataValue> = (&v_list).try_into().unwrap();
        assert_eq!(borrowed_list.len(), 2);
    }

    #[test]
    fn nan_fallback_is_string_and_fails_number_conversion() {
        let v_nan: MetadataValue = f64::NAN.into(); // becomes String variant
        // Expect WrongType because NaN was stored as string
        let err = f64::try_from(v_nan.clone()).unwrap_err();
        match err {
            MetadataConversionError::WrongType { expected, found } => {
                assert_eq!(expected, "Number");
                assert_eq!(found, "String");
            }
            _ => panic!("unexpected error variant for NaN fallback"),
        }
    }

    #[test]
    fn negative_i64_not_u64() {
        let neg: MetadataValue = (-5i64).into();
        let err = u64::try_from(neg).unwrap_err();
        match err {
            MetadataConversionError::NumberOutOfRange { expected, .. } => {
                assert_eq!(expected, "u64");
            }
            _ => panic!("expected NumberOutOfRange for negative to u64"),
        }
    }

    #[test]
    fn round_trip_all_variants() {
        // String
        let s_mv: MetadataValue = "round".into();
        assert_eq!(String::try_from(s_mv.clone()).unwrap(), "round");
        // Number
        let num_mv: MetadataValue = 123i64.into();
        assert_eq!(i64::try_from(num_mv.clone()).unwrap(), 123);
        assert_eq!(u64::try_from(num_mv.clone()).unwrap(), 123u64);
        // List
        let list_mv: MetadataValue = vec!["x", "y"].into();
        let vec_back: Vec<MetadataValue> = list_mv.clone().try_into().unwrap();
        assert_eq!(vec_back.len(), 2);
        // Map
        let mut mm = MetadataMap::new();
        mm.insert("a", 1i64);
        let map_mv: MetadataValue = mm.clone().into();
        let map_back: MetadataMap = map_mv.try_into().unwrap();
        assert_eq!(
            map_back
                .get("a")
                .unwrap()
                .as_number()
                .unwrap()
                .as_i64()
                .unwrap(),
            1
        );
    }
}
