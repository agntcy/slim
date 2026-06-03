// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let proto_file = std::path::Path::new(&manifest_dir).join("proto/v1/commands.proto");
    let repo_proto_dir = std::path::Path::new(&manifest_dir).join("../../proto");

    if !proto_file.exists() || !repo_proto_dir.exists() {
        // Published package: rely on the pre-generated src/gen/ file.
        return Ok(());
    }

    println!("cargo:rerun-if-changed={}", proto_file.display());

    let include_dir = std::path::Path::new(&manifest_dir).join("proto/v1");
    tonic_prost_build::configure()
        .out_dir("src/gen")
        .compile_protos(&[proto_file.to_str().unwrap()], &[include_dir.to_str().unwrap()])?;
    Ok(())
}
