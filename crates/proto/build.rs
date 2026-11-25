// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let base = std::path::Path::new(&manifest_dir);

    // The proto files live in this crate's `proto/` directory, which is part of the
    // published package: the code is generated into OUT_DIR at every build.
    let proto_dir = base.join("proto");

    let dataplane_proto = proto_dir.join("data-plane/v1/data_plane.proto");
    let controller_proto = proto_dir.join("controller/v1/controller.proto");
    let controlplane_proto = proto_dir.join("controlplane/v1/controlplane.proto");
    let channel_manager_proto = proto_dir.join("channel-manager/v1/commands.proto");

    println!("cargo:rerun-if-changed={}", dataplane_proto.display());
    println!("cargo:rerun-if-changed={}", controller_proto.display());
    println!("cargo:rerun-if-changed={}", controlplane_proto.display());
    println!("cargo:rerun-if-changed={}", channel_manager_proto.display());

    // Compile all protos together in a single invocation, into OUT_DIR (a build script
    // must not write anywhere else, and `cargo publish` rejects one that does).
    // Gate tonic client/server modules to non-wasm32 targets.
    tonic_prost_build::configure()
        .client_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .server_mod_attribute(".", "#[cfg(not(target_arch = \"wasm32\"))]")
        .compile_protos(
            &[
                dataplane_proto.to_str().unwrap(),
                controller_proto.to_str().unwrap(),
                controlplane_proto.to_str().unwrap(),
                channel_manager_proto.to_str().unwrap(),
            ],
            &[proto_dir.to_str().unwrap()],
        )
        .unwrap();
}
