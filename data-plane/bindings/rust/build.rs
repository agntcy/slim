// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    slim_version::build::set_git_sha();
    slim_version::build::set_build_date();
    slim_version::build::set_profile();
}
