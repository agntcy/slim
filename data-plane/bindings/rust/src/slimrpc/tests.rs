// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for SlimRPC
//!
//! These tests verify the core functionality of SlimRPC components
//! without requiring full SLIM infrastructure.

use crate::slimrpc::{Code, Metadata, Status};

#[test]
fn test_status_ok() {
    let status = Status::ok();
    assert_eq!(status.code(), Code::Ok);
    assert!(status.is_ok());
    assert_eq!(status.message(), "");
}

#[test]
fn test_status_cancelled() {
    let status = Status::cancelled("Operation cancelled");
    assert_eq!(status.code(), Code::Cancelled);
    assert!(!status.is_ok());
    assert_eq!(status.message(), "Operation cancelled");
}

#[test]
fn test_status_invalid_argument() {
    let status = Status::invalid_argument("Invalid input");
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(status.message(), "Invalid input");
}

#[test]
fn test_status_not_found() {
    let status = Status::not_found("Resource not found");
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(status.message(), "Resource not found");
}

#[test]
fn test_status_internal() {
    let status = Status::internal("Internal error");
    assert_eq!(status.code(), Code::Internal);
    assert_eq!(status.message(), "Internal error");
}

#[test]
fn test_status_unavailable() {
    let status = Status::unavailable("Service unavailable");
    assert_eq!(status.code(), Code::Unavailable);
    assert_eq!(status.message(), "Service unavailable");
}

#[test]
fn test_status_unauthenticated() {
    let status = Status::unauthenticated("Not authenticated");
    assert_eq!(status.code(), Code::Unauthenticated);
    assert_eq!(status.message(), "Not authenticated");
}

#[test]
fn test_status_permission_denied() {
    let status = Status::permission_denied("Access denied");
    assert_eq!(status.code(), Code::PermissionDenied);
    assert_eq!(status.message(), "Access denied");
}

#[test]
fn test_status_new() {
    let status = Status::new(Code::AlreadyExists, "Resource exists");
    assert_eq!(status.code(), Code::AlreadyExists);
    assert_eq!(status.message(), "Resource exists");
}

#[test]
fn test_code_from_i32() {
    assert_eq!(Code::from_i32(0), Some(Code::Ok));
    assert_eq!(Code::from_i32(1), Some(Code::Cancelled));
    assert_eq!(Code::from_i32(2), Some(Code::Unknown));
    assert_eq!(Code::from_i32(3), Some(Code::InvalidArgument));
    assert_eq!(Code::from_i32(4), Some(Code::DeadlineExceeded));
    assert_eq!(Code::from_i32(5), Some(Code::NotFound));
    assert_eq!(Code::from_i32(6), Some(Code::AlreadyExists));
    assert_eq!(Code::from_i32(7), Some(Code::PermissionDenied));
    assert_eq!(Code::from_i32(8), Some(Code::ResourceExhausted));
    assert_eq!(Code::from_i32(9), Some(Code::FailedPrecondition));
    assert_eq!(Code::from_i32(10), Some(Code::Aborted));
    assert_eq!(Code::from_i32(11), Some(Code::OutOfRange));
    assert_eq!(Code::from_i32(12), Some(Code::Unimplemented));
    assert_eq!(Code::from_i32(13), Some(Code::Internal));
    assert_eq!(Code::from_i32(14), Some(Code::Unavailable));
    assert_eq!(Code::from_i32(15), Some(Code::DataLoss));
    assert_eq!(Code::from_i32(16), Some(Code::Unauthenticated));
    assert_eq!(Code::from_i32(999), None);
    assert_eq!(Code::from_i32(-1), None);
}

#[test]
fn test_code_as_i32() {
    assert_eq!(Code::Ok.as_i32(), 0);
    assert_eq!(Code::Cancelled.as_i32(), 1);
    assert_eq!(Code::Unknown.as_i32(), 2);
    assert_eq!(Code::InvalidArgument.as_i32(), 3);
    assert_eq!(Code::DeadlineExceeded.as_i32(), 4);
    assert_eq!(Code::NotFound.as_i32(), 5);
    assert_eq!(Code::AlreadyExists.as_i32(), 6);
    assert_eq!(Code::PermissionDenied.as_i32(), 7);
    assert_eq!(Code::ResourceExhausted.as_i32(), 8);
    assert_eq!(Code::FailedPrecondition.as_i32(), 9);
    assert_eq!(Code::Aborted.as_i32(), 10);
    assert_eq!(Code::OutOfRange.as_i32(), 11);
    assert_eq!(Code::Unimplemented.as_i32(), 12);
    assert_eq!(Code::Internal.as_i32(), 13);
    assert_eq!(Code::Unavailable.as_i32(), 14);
    assert_eq!(Code::DataLoss.as_i32(), 15);
    assert_eq!(Code::Unauthenticated.as_i32(), 16);
}

#[test]
fn test_metadata_new() {
    let metadata = Metadata::new();
    assert!(metadata.is_empty());
    assert_eq!(metadata.len(), 0);
}

#[test]
fn test_metadata_insert_and_get() {
    let mut metadata = Metadata::new();
    
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    assert_eq!(metadata.len(), 2);
    assert!(!metadata.is_empty());
    
    assert_eq!(metadata.get("key1"), Some(&"value1".to_string()));
    assert_eq!(metadata.get("key2"), Some(&"value2".to_string()));
    assert_eq!(metadata.get("key3"), None);
}

#[test]
fn test_metadata_contains_key() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    
    assert!(metadata.contains_key("key1"));
    assert!(!metadata.contains_key("key2"));
}

#[test]
fn test_metadata_remove() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    assert_eq!(metadata.remove("key1"), Some("value1".to_string()));
    assert_eq!(metadata.remove("key1"), None);
    assert_eq!(metadata.len(), 1);
    assert!(!metadata.contains_key("key1"));
    assert!(metadata.contains_key("key2"));
}

#[test]
fn test_metadata_clear() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    metadata.clear();
    assert!(metadata.is_empty());
    assert_eq!(metadata.len(), 0);
}

#[test]
fn test_metadata_merge() {
    let mut metadata1 = Metadata::new();
    metadata1.insert("key1", "value1");
    metadata1.insert("key2", "value2");
    
    let mut metadata2 = Metadata::new();
    metadata2.insert("key2", "new_value2");
    metadata2.insert("key3", "value3");
    
    metadata1.merge(metadata2);
    
    assert_eq!(metadata1.len(), 3);
    assert_eq!(metadata1.get("key1").map(|s| s.as_ref()), Some("value1"));
    assert_eq!(metadata1.get("key2").map(|s| s.as_ref()), Some("new_value2")); // Overwritten
    assert_eq!(metadata1.get("key3").map(|s| s.as_ref()), Some("value3"));
}

#[test]
fn test_metadata_from_map() {
    let mut map = std::collections::HashMap::new();
    map.insert("key1".to_string(), "value1".to_string());
    map.insert("key2".to_string(), "value2".to_string());
    
    let metadata = Metadata::from_map(map.clone());
    
    assert_eq!(metadata.len(), 2);
    assert_eq!(metadata.get("key1").map(|s| s.as_ref()), Some("value1"));
    assert_eq!(metadata.get("key2").map(|s| s.as_ref()), Some("value2"));
}

#[test]
fn test_metadata_as_map() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    let _map = metadata.as_map();
    assert_eq!(metadata.len(), 2);
    assert_eq!(metadata.get("key1").map(|s| s.as_ref()), Some("value1"));
    assert_eq!(metadata.get("key2").map(|s| s.as_ref()), Some("value2"));
}

#[test]
fn test_metadata_iter() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    let mut count = 0;
    for (key, value) in metadata.iter() {
        assert!(key == "key1" || key == "key2");
        assert!(value == "value1" || value == "value2");
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn test_constants() {
    use crate::slimrpc::{DEADLINE_KEY, STATUS_CODE_KEY, MAX_TIMEOUT};
    
    assert_eq!(DEADLINE_KEY, "slimrpc-deadline");
    assert_eq!(STATUS_CODE_KEY, "slimrpc-code");
    assert_eq!(MAX_TIMEOUT, 36000);
}

#[test]
fn test_status_display() {
    let status = Status::new(Code::InvalidArgument, "Bad request");
    let display = format!("{}", status);
    assert!(display.contains("InvalidArgument"));
    assert!(display.contains("Bad request"));
}

#[test]
fn test_code_display() {
    assert_eq!(format!("{}", Code::Ok), "Ok");
    assert_eq!(format!("{}", Code::Cancelled), "Cancelled");
    assert_eq!(format!("{}", Code::InvalidArgument), "InvalidArgument");
    assert_eq!(format!("{}", Code::NotFound), "NotFound");
}

#[test]
fn test_metadata_update_existing() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    
    metadata.insert("key1", "updated_value1");
    
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata.get("key1").map(|s| s.as_ref()), Some("updated_value1"));
}

#[test]
fn test_metadata_keys() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    metadata.insert("key3", "value3");
    
    let keys: Vec<_> = metadata.keys().collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().any(|k| k.as_ref() == "key1"));
    assert!(keys.iter().any(|k| k.as_ref() == "key2"));
    assert!(keys.iter().any(|k| k.as_ref() == "key3"));
}

#[test]
fn test_metadata_values() {
    let mut metadata = Metadata::new();
    metadata.insert("key1", "value1");
    metadata.insert("key2", "value2");
    
    let values: Vec<_> = metadata.values().collect();
    assert_eq!(values.len(), 2);
    assert!(values.iter().any(|v| *v == "value1"));
    assert!(values.iter().any(|v| *v == "value2"));
}