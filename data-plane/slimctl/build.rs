// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    slim_version::build::setup("slimctl-v*");

    // Compile proto files
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let proto_dir = std::path::Path::new(&manifest_dir).join("proto");

    let controlplane_proto = proto_dir.join("controlplane/v1/controlplane.proto");
    let controller_proto = proto_dir.join("controller/v1/controller.proto");
    let channel_manager_proto = proto_dir.join("channel_manager/v1/commands.proto");

    let datapath_proto_dir = std::path::Path::new(&manifest_dir).join("../../proto");

    // When building from a published package the symlinked proto files are
    // resolved but the repo-level proto directory (needed for the
    // data-plane/v1/data_plane.proto import) is absent.  Skip compilation
    // and rely on pre-generated code in that case.
    if !controlplane_proto.exists()
        || !controller_proto.exists()
        || !channel_manager_proto.exists()
        || !datapath_proto_dir.exists()
    {
        return;
    }

    println!("cargo:rerun-if-changed={}", controlplane_proto.display());
    println!("cargo:rerun-if-changed={}", controller_proto.display());
    println!("cargo:rerun-if-changed={}", channel_manager_proto.display());

    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .extern_path(
            ".dataplane.proto.v1",
            "::slim_datapath::api::proto::dataplane::v1",
        )
        .compile_protos(
            &[
                controlplane_proto.to_str().unwrap(),
                controller_proto.to_str().unwrap(),
                channel_manager_proto.to_str().unwrap(),
            ],
            &[
                proto_dir.to_str().unwrap(),
                datapath_proto_dir.to_str().unwrap(),
            ],
        )
        .unwrap();
}
