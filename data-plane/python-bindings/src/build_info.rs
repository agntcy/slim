// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agp_config::buildinfo::BuildInfo;

pub const BUILD_INFO: BuildInfo = BuildInfo {
    date: env!("BUILD_DATE"),
    git_sha: env!("GIT_SHA"),
    profile: env!("PROFILE"),
    version: env!("VERSION"),
};
