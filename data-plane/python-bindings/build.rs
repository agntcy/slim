// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agp_config::buildinfo;

fn main() {
    buildinfo::fill("agp-bindings-v*");
}
