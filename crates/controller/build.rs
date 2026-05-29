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

    // The canonical controller.proto lives in this crate at
    // proto/controller/v1/controller.proto.
    //
    // The generated src/api/gen/controller.proto.v1.rs is committed to the
    // repository.  When building from a published package the pre-generated
    // file is used as-is and this build script skips proto compilation.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let base = std::path::Path::new(&manifest_dir);
    let proto_file = base.join("proto/controller/v1/controller.proto");

    if !proto_file.exists() {
        // Published package: rely on the pre-generated src/api/gen/ file.
        return;
    }

    println!("cargo:rerun-if-changed={}", proto_file.display());

    // Include paths: local proto/ for controller, sibling datapath proto/ for data-plane imports
    let datapath_proto = base.join("../datapath/proto");

    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .extern_path(
            ".dataplane.proto.v1",
            "::slim_datapath::api::proto::dataplane::v1",
        )
        .compile_protos(
            &[proto_file.to_str().unwrap()],
            &[
                base.join("proto").to_str().unwrap(),
                datapath_proto.to_str().unwrap(),
            ],
        )
        .unwrap();
}
