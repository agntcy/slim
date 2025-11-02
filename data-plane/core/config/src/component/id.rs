// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use regex::Regex;
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IdError {
    #[error("id cannot be emapty")]
    Empty,
    #[error("kind contains invalid character(s): {0}")]
    InvalidCharacter(String),
    #[error("name part is too long: {0}")]
    NameTooLong(String),
    #[error("unknown error")]
    Unknown,
}

// Constant for the separator used in composite keys
const KIND_AND_NAME_SEPARATOR: &str = "/";

// Regex patterns for validating kind and names
lazy_static::lazy_static! {
    static ref KIND_REGEX: Regex = Regex::new(r"^[a-zA-Z][0-9a-zA-Z_]{0,62}$").unwrap();
    static ref NAME_REGEX: Regex = Regex::new(r"^[a-z0-9-]+$").unwrap();
}

/// Kind represents the type of a component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Kind {
    name: String,
}

impl Kind {
    /// Create a new Kind
    pub fn new(ty: &str) -> Result<Self, IdError> {
        if ty.is_empty() {
            return Err(IdError::Empty);
        }
        if !KIND_REGEX.is_match(ty) {
            return Err(IdError::InvalidCharacter(ty.to_string()));
        }
        Ok(Kind {
            name: ty.to_string(),
        })
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// ID represents a unique identifier for a component.
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct ID {
    kind_val: Kind,
    name_val: String,
}

impl ID {
    /// Create a new ID
    pub fn new(kind_val: Kind) -> Self {
        ID {
            kind_val,
            name_val: String::new(),
        }
    }

    /// Create a new ID with a name
    pub fn new_with_name(kind_val: Kind, name_val: &str) -> Result<Self, IdError> {
        validate_name(name_val)?;
        Ok(ID {
            kind_val,
            name_val: name_val.to_string(),
        })
    }

    pub fn new_with_str(kind_and_name: &str) -> Result<Self, IdError> {
        let (kind, name) = kind_and_name
            .split_once(KIND_AND_NAME_SEPARATOR)
            .unwrap_or(("", ""));

        if kind.is_empty() {
            return Err(IdError::Empty);
        }

        let kind_val = Kind::new(kind)?;

        ID::new_with_name(kind_val, name)
    }

    /// Get the kind of the ID
    pub fn kind(&self) -> &Kind {
        &self.kind_val
    }

    /// Get the name of the ID
    pub fn name(&self) -> &str {
        &self.name_val
    }

    /// Marshal the ID to a string
    pub fn marshal_text(&self) -> String {
        self.to_string()
    }
}

/// Implement Display for ID
impl fmt::Display for ID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.name_val.is_empty() {
            write!(f, "{}", self.kind_val)
        } else {
            write!(
                f,
                "{}{}{}",
                self.kind_val, KIND_AND_NAME_SEPARATOR, self.name_val
            )
        }
    }
}

/// Validate the name part of an ID
fn validate_name(name_str: &str) -> Result<(), IdError> {
    // it is ok if the name is empty
    if name_str.is_empty() {
        return Ok(());
    }

    if name_str.len() > 1024 {
        return Err(IdError::NameTooLong(name_str.to_string()));
    }
    if !NAME_REGEX.is_match(name_str) {
        return Err(IdError::InvalidCharacter(name_str.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_creation() {
        let valid_kind = Kind::new("validKind").unwrap();
        assert_eq!(valid_kind.to_string(), "validKind");

        assert!(Kind::new("").is_err());
        assert!(Kind::new("123Invalid").is_err());
    }

    #[test]
    fn test_id_creation() {
        let kind_val = Kind::new("validKind").unwrap();

        let id = ID::new(kind_val.clone());
        assert_eq!(id.kind().to_string(), "validKind");
        assert!(id.name().is_empty());

        let id_with_name = ID::new_with_name(kind_val.clone(), "valid-name").unwrap();
        assert_eq!(id_with_name.name(), "valid-name");

        assert!(ID::new_with_name(kind_val.clone(), "").is_ok());
        assert!(ID::new_with_name(kind_val.clone(), "Invalid Name!").is_err());
    }
}
