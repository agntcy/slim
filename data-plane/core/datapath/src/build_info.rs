// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use semver::Version;
use std::cmp::Ordering;
use std::sync::OnceLock;

pub const BUILD_INFO: BuildInfo = BuildInfo {
    date: env!("BUILD_DATE"),
    git_sha: env!("GIT_SHA"),
    profile: env!("PROFILE"),
    version: env!("VERSION"),
};

/// Cached version string, created only once on first access.
/// This avoids allocating a new String for every packet.
static VERSION_STRING: OnceLock<String> = OnceLock::new();

/// Get the version string, creating it only once.
/// Returns a reference to the cached version string.
pub fn version_string() -> &'static String {
    VERSION_STRING.get_or_init(|| BUILD_INFO.version.to_string())
}

#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct BuildInfo {
    pub date: &'static str,
    pub git_sha: &'static str,
    pub profile: &'static str,
    pub version: &'static str,
}

impl BuildInfo {
    /// Parse version string into a semver Version.
    /// Handles formats from `git describe --tags --always --match "slim-v*"`:
    /// - "slim-v1.2.3" (exact tag match)
    /// - "slim-v1.2.3-30-g26599324" (30 commits after tag slim-v1.2.3, at commit g26599324)
    /// - "26599324" (commit hash only when no tags exist)
    /// 
    /// For version comparison, only the semver part (1.2.3) is used, ignoring commit count and hash.
    pub fn parse_version(version_str: &str) -> Option<Version> {
        let version_str = version_str.trim();
        
        // Strip "slim-v" prefix if present
        let version_str = version_str
            .strip_prefix("slim-v")
            .unwrap_or(version_str);
        
        // Handle "v1.2.3-10-gabcdef" format - take only the semver part
        let semver_part = if let Some(dash_pos) = version_str.find('-') {
            // Check if what follows is a number (commit count)
            let after_dash = &version_str[dash_pos + 1..];
            if after_dash.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                &version_str[..dash_pos]
            } else {
                version_str
            }
        } else {
            version_str
        };
        
        // Try to parse as semver
        Version::parse(semver_part).ok()
    }
    
    /// Compare current version with another version string.
    /// Returns:
    /// - Some(Ordering::Equal) if versions are equal
    /// - Some(Ordering::Greater) if current version is newer
    /// - Some(Ordering::Less) if current version is older
    /// - None if either version cannot be parsed (e.g., legacy versions)
    pub fn compare_version(&self, other: Option<&str>) -> Option<Ordering> {
        let current_version = Self::parse_version(self.version)?;
        let other_str = other?;
        let other_version = Self::parse_version(other_str)?;
        
        Some(current_version.cmp(&other_version))
    }
    
    /// Check if another version is compatible (same major version for semver).
    /// Returns None if either version cannot be parsed.
    pub fn is_compatible_with(&self, other: Option<&str>) -> Option<bool> {
        let current_version = Self::parse_version(self.version)?;
        let other_str = other?;
        let other_version = Self::parse_version(other_str)?;
        
        Some(current_version.major == other_version.major)
    }
    
    /// Check if the current version is newer than or equal to another version.
    /// Returns true if current >= other, false otherwise.
    /// Returns None if versions cannot be compared.
    pub fn is_at_least(&self, other: Option<&str>) -> Option<bool> {
        let ordering = self.compare_version(other)?;
        Some(matches!(ordering, Ordering::Greater | Ordering::Equal))
    }
}

impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Version:\t{}\nBuild Date:\t{}\nGit SHA:\t{}\nProfile:\t{}",
            self.version, self.date, self.git_sha, self.profile
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        // Exact tag
        let v = BuildInfo::parse_version("slim-v1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        
        // Tag with commits after
        let v = BuildInfo::parse_version("slim-v1.2.3-10-gabcdef").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        
        // Without prefix
        let v = BuildInfo::parse_version("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }
    
    #[test]
    fn test_compare_version() {
        let build_info = BuildInfo {
            date: "",
            git_sha: "",
            profile: "",
            version: "slim-v1.2.3",
        };
        
        // Equal versions
        assert_eq!(
            build_info.compare_version(Some("slim-v1.2.3")),
            Some(Ordering::Equal)
        );
        
        // Current is newer
        assert_eq!(
            build_info.compare_version(Some("slim-v1.2.2")),
            Some(Ordering::Greater)
        );
        
        // Current is older
        assert_eq!(
            build_info.compare_version(Some("slim-v1.2.4")),
            Some(Ordering::Less)
        );
        
        // With commits after tag
        assert_eq!(
            build_info.compare_version(Some("slim-v1.2.3-5-gabcdef")),
            Some(Ordering::Equal)
        );
        
        // None for unparseable version
        assert_eq!(build_info.compare_version(Some("invalid")), None);
        
        // None for missing version (legacy)
        assert_eq!(build_info.compare_version(None), None);
    }
    
    #[test]
    fn test_is_compatible() {
        let build_info = BuildInfo {
            date: "",
            git_sha: "",
            profile: "",
            version: "slim-v1.2.3",
        };
        
        // Same major version
        assert_eq!(build_info.is_compatible_with(Some("slim-v1.5.0")), Some(true));
        
        // Different major version
        assert_eq!(build_info.is_compatible_with(Some("slim-v2.0.0")), Some(false));
        
        // None for unparseable
        assert_eq!(build_info.is_compatible_with(Some("invalid")), None);
    }
    
    #[test]
    fn test_is_at_least() {
        let build_info = BuildInfo {
            date: "",
            git_sha: "",
            profile: "",
            version: "slim-v1.2.3",
        };
        
        // Current is equal
        assert_eq!(build_info.is_at_least(Some("slim-v1.2.3")), Some(true));
        
        // Current is newer
        assert_eq!(build_info.is_at_least(Some("slim-v1.2.2")), Some(true));
        
        // Current is older
        assert_eq!(build_info.is_at_least(Some("slim-v1.2.4")), Some(false));
    }
    
    #[test]
    fn test_version_string_caching() {
        // Get the version string twice
        let v1 = version_string();
        let v2 = version_string();
        
        // Verify they point to the same memory location (cached)
        assert_eq!(v1.as_ptr(), v2.as_ptr());
        
        // Verify the content is correct
        assert_eq!(v1, BUILD_INFO.version);
    }
}
