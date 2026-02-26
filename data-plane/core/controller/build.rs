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

    // The canonical source for controller.proto lives at
    // proto/controller/v1/controller.proto in the repository root.
    // This crate's proto/v1/controller.proto is a symlink that points there,
    // allowing other crates and tools to reference the file from a shared
    // location while keeping a local path for proto compilation.
    //
    // The generated src/api/gen/controller.proto.v1.rs is committed to the
    // repository.  When building from a published package (where the
    // workspace and its symlinks are unavailable) the pre-generated file is
    // used as-is and this build script skips proto compilation.
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
