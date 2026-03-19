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

    println!("cargo:rerun-if-changed={}", controlplane_proto.display());
    println!("cargo:rerun-if-changed={}", controller_proto.display());

    tonic_prost_build::configure()
        .compile_protos(
            &[
                controlplane_proto.to_str().unwrap(),
                controller_proto.to_str().unwrap(),
            ],
            &[proto_dir.to_str().unwrap()],
        )
        .unwrap();
}
