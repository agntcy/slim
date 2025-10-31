// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    // Get protoc path
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    // export PROTOC to the environment
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .compile_protos(&["proto/v1/controller.proto"], &["proto/v1"])
        .unwrap();
}
