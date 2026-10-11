// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub const BUILD_INFO: BuildInfo = BuildInfo {
    date: env!("BUILD_DATE"),
    git_sha: env!("GIT_SHA"),
    profile: env!("PROFILE"),
    version: env!("CARGO_PKG_VERSION"),
};

#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct BuildInfo {
    pub date: &'static str,
    pub git_sha: &'static str,
    pub profile: &'static str,
    pub version: &'static str,
}

// to string
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
    fn version_is_the_crate_version() {
        assert_eq!(BUILD_INFO.version, env!("CARGO_PKG_VERSION"));
    }
}
