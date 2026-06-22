// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/// Return the current SLIM version.
///
/// This is in sync with the crate version defined in Cargo.toml.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Return the git SHA of the build.
pub fn git_sha() -> &'static str {
    env!("GIT_SHA")
}

/// Return the build date in ISO 8601 UTC format.
pub fn build_date() -> &'static str {
    env!("BUILD_DATE")
}

/// Return the build profile (e.g. `debug` or `release`).
pub fn profile() -> &'static str {
    env!("PROFILE")
}

/// Structured build information.
#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub date: &'static str,
    pub profile: &'static str,
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

/// Return a [`BuildInfo`] populated with compile-time values.
pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: version(),
        git_sha: git_sha(),
        date: build_date(),
        profile: profile(),
    }
}

/// Helpers for use inside `build.rs` scripts.
///
/// Add `agntcy-slim-version` to `[build-dependencies]` to use these.
pub mod build {
    use std::process::Command;

    /// Run `cmd`, trim its stdout, and emit `cargo:rustc-env=NAME=<value>`.
    ///
    /// On error, emits a `cargo:warning` and uses an empty string.
    pub fn set_env(name: &str, cmd: &mut Command) {
        let value = match cmd.output() {
            Ok(output) => String::from_utf8(output.stdout)
                .unwrap_or_default()
                .trim()
                .to_string(),
            Err(err) => {
                println!("cargo:warning={err}");
                String::new()
            }
        };
        println!("cargo:rustc-env={name}={value}");
    }

    /// Emit `cargo:rustc-env=GIT_SHA=<short hash>`.
    pub fn set_git_sha() {
        set_env(
            "GIT_SHA",
            Command::new("git").args(["rev-parse", "--short", "HEAD"]),
        );
    }

    /// Emit `cargo:rustc-env=BUILD_DATE=<ISO 8601 UTC timestamp>`.
    pub fn set_build_date() {
        set_env(
            "BUILD_DATE",
            Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
        );
    }

    /// Emit `cargo:rustc-env=VERSION=<git describe>` matching `tag_pattern`.
    pub fn set_version(tag_pattern: &str) {
        set_env(
            "VERSION",
            Command::new("git").args(["describe", "--tags", "--always", "--match", tag_pattern]),
        );
    }

    /// Emit `cargo:rustc-env=PROFILE=<debug|release>`.
    pub fn set_profile() {
        let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
        println!("cargo:rustc-env=PROFILE={profile}");
    }

    /// Emit all standard SLIM build environment variables.
    ///
    /// Sets `GIT_SHA`, `BUILD_DATE`, `VERSION` (matched against `tag_pattern`), and `PROFILE`.
    pub fn setup(tag_pattern: &str) {
        set_git_sha();
        set_build_date();
        set_version(tag_pattern);
        set_profile();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_version() {
        assert_eq!(version().to_string(), env!("CARGO_PKG_VERSION").to_string());
    }

    #[test]
    fn test_build_info() {
        let info = build_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.git_sha.is_empty());
        assert!(!info.date.is_empty());
        assert!(!info.profile.is_empty());
    }
}
