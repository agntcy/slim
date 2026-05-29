// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs;
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
    tonic_prost_build::configure()
        .out_dir(out_dir)
        .compile_protos(
            &[proto_file.to_str().unwrap()],
            &[include_dir.to_str().unwrap()],
        )
        .unwrap();

    // Post-process: the generated `*_client` / `*_server` modules pull in
    // `tonic::transport::*` and `tonic::codegen::*`, which need hyper/h2/tokio
    // net and are not available on wasm32-unknown-unknown. Gate those modules
    // to non-wasm32 targets so the rest of the proto messages (pure prost) can
    // still be used from a browser build.
    let gen_file = out_dir.join("dataplane.proto.v1.rs");
    let src = fs::read_to_string(&gen_file).expect("read generated proto file");
    let patched = src
        .replace(
            "/// Generated client implementations.\npub mod data_plane_service_client {",
            "/// Generated client implementations.\n#[cfg(not(target_arch = \"wasm32\"))]\npub mod data_plane_service_client {",
        )
        .replace(
            "/// Generated server implementations.\npub mod data_plane_service_server {",
            "/// Generated server implementations.\n#[cfg(not(target_arch = \"wasm32\"))]\npub mod data_plane_service_server {",
        );
    if patched != src {
        fs::write(&gen_file, patched).expect("write patched proto file");
    }
}
