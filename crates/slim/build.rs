// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// The version comes from CARGO_PKG_VERSION, not `git describe`: release
// builds use a shallow checkout without tags, and `cargo install` and the
// container builds have no git repository at all.
fn main() {
    slim_version::build::set_git_sha();
    slim_version::build::set_build_date();
    slim_version::build::set_profile();
}
