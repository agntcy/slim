// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    uniffi::generate_scaffolding("src/slim_bindings.udl").unwrap();
}

