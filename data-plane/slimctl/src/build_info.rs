// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub const BUILD_INFO: BuildInfo = BuildInfo {
    date: env!("BUILD_DATE"),
    git_sha: env!("GIT_SHA"),
    profile: env!("PROFILE"),
    version: env!("VERSION"),
};

#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct BuildInfo {
    pub date: &'static str,
    pub git_sha: &'static str,
    pub profile: &'static str,
    pub version: &'static str,
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

    fn test_info() -> BuildInfo {
        BuildInfo {
            date: "2024-01-01",
            git_sha: "abc123def",
            profile: "release",
            version: "1.2.3",
        }
    }

    #[test]
    fn display_contains_all_fields() {
        let s = test_info().to_string();
        assert!(s.contains("1.2.3"));
        assert!(s.contains("2024-01-01"));
        assert!(s.contains("abc123def"));
        assert!(s.contains("release"));
    }

    #[test]
    fn display_uses_tab_separated_labels() {
        let s = test_info().to_string();
        assert!(s.contains("Version:\t1.2.3"));
        assert!(s.contains("Build Date:\t2024-01-01"));
        assert!(s.contains("Git SHA:\tabc123def"));
        assert!(s.contains("Profile:\trelease"));
    }

    #[test]
    fn equality_reflexive() {
        let info = test_info();
        assert_eq!(info, info);
    }

    #[test]
    fn equality_different_values() {
        let a = test_info();
        let b = BuildInfo {
            version: "2.0.0",
            ..a
        };
        assert_ne!(a, b);
    }

    #[test]
    fn copy_and_clone() {
        let a = test_info();
        let b = a; // Copy
        let c = a;
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
}
