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
