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

    // The canonical source for controller.proto is this crate's own
    // proto/v1/controller.proto; proto/<repo-root>/controller/v1/ holds a
    // symlink that points here so that other crates and tools can reference it
    // from a shared location.
    //
    // The generated src/api/gen/controller.proto.v1.rs is committed to the
    // repository.  When building from a published package (where the
    // workspace is unavailable) the pre-generated file is used as-is and this
    // build script skips proto compilation.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let proto_file = std::path::Path::new(&manifest_dir).join("proto/v1/controller.proto");

    if !proto_file.exists() {
        // Published package: rely on the pre-generated src/api/gen/ file.
        return;
    }

    println!("cargo:rerun-if-changed={}", proto_file.display());

    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .compile_protos(
            &[proto_file.to_str().unwrap()],
            &[std::path::Path::new(&manifest_dir)
                .join("proto/v1")
                .to_str()
                .unwrap()],
        )
        .unwrap();
}
