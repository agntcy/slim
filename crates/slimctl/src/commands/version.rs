// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::build_info;

pub fn run() {
    println!("{}", build_info::BUILD_INFO);
}
