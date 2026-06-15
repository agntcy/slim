// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

fn main() {
    // Get protoc path
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    // export PROTOC to the environment
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let proto_file = std::path::Path::new(&manifest_dir).join("proto/v1/data_plane.proto");
    let repo_proto_dir = std::path::Path::new(&manifest_dir).join("../../../proto");

    if !proto_file.exists() || !repo_proto_dir.exists() {
        // Published package: rely on the pre-generated src/api/gen/ file.
        return;
    }

    println!("cargo:rerun-if-changed={}", proto_file.display());

    let include_dir = std::path::Path::new(&manifest_dir).join("proto/v1");
    let out_dir = Path::new("src/api/gen");

    // Gate the generated client/server modules to non-wasm32 targets: they
    // depend on tonic::transport (hyper/h2/tokio-net) which is unavailable on
    // wasm32-unknown-unknown. The proto message types (pure prost) remain
    // available on all targets.
    tonic_prost_build::configure()
        .out_dir(out_dir)
        .client_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .server_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .compile_protos(
            &[proto_file.to_str().unwrap()],
            &[include_dir.to_str().unwrap()],
        )
        .unwrap();
}
