fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("failed to locate protoc");

    let proto_root = std::path::PathBuf::from("../../proto");
    let controlplane_proto = proto_root.join("controlplane/v1/controlplane.proto");
    let controller_proto = proto_root.join("controller/v1/controller.proto");

    let controlplane_proto_str = controlplane_proto
        .to_str()
        .expect("invalid controlplane proto path");
    let controller_proto_str = controller_proto
        .to_str()
        .expect("invalid controller proto path");
    let proto_root_str = proto_root.to_str().expect("invalid proto include path");

    println!("cargo:rerun-if-changed={controlplane_proto_str}");
    println!("cargo:rerun-if-changed={controller_proto_str}");

    let mut prost_config = tonic_prost_build::Config::new();
    prost_config.protoc_executable(protoc_path);

    tonic_prost_build::configure()
        .build_server(false)
        .compile_with_config(
            prost_config,
            &[controlplane_proto_str, controller_proto_str],
            &[proto_root_str],
        )
        .expect("failed to compile protobuf definitions for slimctl");
}
