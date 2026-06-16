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

    // Gate the generated client/server modules to non-wasm32 targets: they
    // depend on tonic::transport (hyper/h2/tokio-net) which is unavailable on
    // wasm32-unknown-unknown. The proto message types (pure prost) remain
    // available on all targets.
    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .client_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .server_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .compile_protos(&["proto/data-plane/v1/data_plane.proto"], &["proto"])
        .unwrap();
}
