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
        // Gate the tonic gRPC service modules behind the "native" feature so the
        // generated proto file compiles on wasm32 targets where tonic is unavailable.
        .server_mod_attribute("dataplane.proto.v1", "#[cfg(feature = \"native\")]")
        .client_mod_attribute("dataplane.proto.v1", "#[cfg(feature = \"native\")]")
        .compile_protos(&["proto/v1/data_plane.proto"], &["proto/v1"])
        .unwrap();
}
