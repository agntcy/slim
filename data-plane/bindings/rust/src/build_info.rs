// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/// Build information for the SLIM bindings
#[derive(Debug, Clone, uniffi::Record)]
pub struct BuildInfo {
    /// Semantic version (e.g., "0.7.0")
    pub version: String,
    /// Git commit hash (short)
    pub git_sha: String,
    /// Build date in ISO 8601 UTC format
    pub build_date: String,
    /// Build profile (debug/release)
    pub profile: String,
}

/// Get the version of the SLIM bindings (simple string)
#[uniffi::export]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get detailed build information
#[uniffi::export]
pub fn get_build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: env!("GIT_SHA").to_string(),
        build_date: env!("BUILD_DATE").to_string(),
        profile: env!("PROFILE").to_string(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test get_version returns non-empty version string
    #[test]
    fn test_get_version() {
        let version = get_version();
        assert!(!version.is_empty());
        // Version should be semver format
        assert!(
            version.contains('.'),
            "Version should contain dot separator"
        );
    }

    /// Test get_build_info returns valid build information
    #[test]
    fn test_get_build_info() {
        let info = get_build_info();

        assert!(!info.version.is_empty());
        assert!(!info.git_sha.is_empty());
        assert!(!info.build_date.is_empty());
        assert!(!info.profile.is_empty());

        // Profile should be either debug or release
        assert!(
            info.profile == "debug" || info.profile == "release",
            "Profile should be debug or release, got: {}",
            info.profile
        );
    }

    /// Test BuildInfo clone and debug
    #[test]
    fn test_build_info_traits() {
        let info = get_build_info();
        let cloned = info.clone();

        assert_eq!(info.version, cloned.version);
        assert_eq!(info.git_sha, cloned.git_sha);
        assert_eq!(info.build_date, cloned.build_date);
        assert_eq!(info.profile, cloned.profile);

        // Debug trait should work
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("BuildInfo"));
    }
}
