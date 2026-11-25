// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    // export PROTOC to the environment
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    // Generate into OUT_DIR: a build script must not write anywhere else, and `cargo
    // publish` rejects one that does.
    tonic_prost_build::configure()
        .compile_protos(&["proto/hello.proto"], &["proto"])
        .unwrap();
}
