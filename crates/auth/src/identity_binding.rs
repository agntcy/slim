// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! How strictly a peer's identity is held to the key and name it uses.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Environment variable that sets the process-wide [`IdentityBinding`].
pub const IDENTITY_BINDING_ENV: &str = "SLIM_IDENTITY_BINDING";

/// What to do when a peer's identity isn't bound to what it presents, for
/// example a token whose `sub` doesn't belong to the trusted key that signed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IdentityBinding {
    /// Don't check.
    Off,
    /// Check, and log a warning on a mismatch.
    #[default]
    Warn,
    /// Check, and reject on a mismatch.
    Enforce,
}

impl IdentityBinding {
    /// The process-wide setting: [`IDENTITY_BINDING_ENV`] (`off`, `warn` or
    /// `enforce`), read once, or `warn` when it is unset or unrecognized.
    pub fn from_env() -> Self {
        static MODE: OnceLock<IdentityBinding> = OnceLock::new();
        *MODE.get_or_init(|| {
            let value = std::env::var(IDENTITY_BINDING_ENV).ok();
            Self::parse(value.as_deref()).unwrap_or_else(|| {
                tracing::warn!(
                    value = ?value,
                    "unrecognized {IDENTITY_BINDING_ENV}, using warn"
                );
                Self::Warn
            })
        })
    }

    /// `None` for an unrecognized value; an unset value is the default.
    fn parse(value: Option<&str>) -> Option<Self> {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") => Some(Self::default()),
            Some("off") => Some(Self::Off),
            Some("warn") => Some(Self::Warn),
            Some("enforce") => Some(Self::Enforce),
            Some(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_and_defaults_to_warn() {
        assert_eq!(IdentityBinding::parse(None), Some(IdentityBinding::Warn));
        assert_eq!(
            IdentityBinding::parse(Some("")),
            Some(IdentityBinding::Warn)
        );
        assert_eq!(
            IdentityBinding::parse(Some("off")),
            Some(IdentityBinding::Off)
        );
        assert_eq!(
            IdentityBinding::parse(Some(" Warn ")),
            Some(IdentityBinding::Warn)
        );
        assert_eq!(
            IdentityBinding::parse(Some("ENFORCE")),
            Some(IdentityBinding::Enforce)
        );
        assert_eq!(IdentityBinding::parse(Some("strict")), None);
    }
}
