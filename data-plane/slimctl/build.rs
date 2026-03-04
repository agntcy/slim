// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

fn set_env(name: &str, cmd: &mut Command) {
    let value = match cmd.output() {
        Ok(output) => String::from_utf8(output.stdout).unwrap_or_default(),
        Err(err) => {
            println!("cargo:warning={}", err);
            "".to_string()
        }
    };
    println!("cargo:rustc-env={}={}", name, value.trim());
}

fn main() {
    // Set build info environment variables
    set_env(
        "GIT_SHA",
        Command::new("git").args(["rev-parse", "--short", "HEAD"]),
    );
    set_env(
        "BUILD_DATE",
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
    );
    set_env(
        "VERSION",
        Command::new("git").args(["describe", "--tags", "--always", "--match", "slimctl-v*"]),
    );

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=PROFILE={profile}");

    // Compile proto files
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    // Locate the proto directory.
    //
    // During normal workspace development the canonical files live at
    // <repo-root>/proto/ (three levels above data-plane/core/slimctl/).
    // The crate-local proto/ directory holds symlinks that point there, so
    // both strategies ultimately read the same files.
    //
    // When running `cargo publish` the crate is packaged without the rest of
    // the repository.  Cargo resolves symlinks when creating the tarball, so
    // the published package contains the real proto files under proto/.
    // build.rs therefore always uses the crate-local proto/ as its input; no
    // separate copy step is required.
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
