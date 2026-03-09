// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

fn set_env(name: &str, cmd: &mut Command) {
    let value = match cmd.output() {
        Ok(output) => String::from_utf8(output.stdout).unwrap(),
        Err(err) => {
            println!("cargo:warning={}", err);
            "".to_string()
        }
    };
    println!("cargo:rustc-env={}={}", name, value);
}

fn main() {
    // Generate build info
    set_env(
        "GIT_SHA",
        Command::new("git").args(["rev-parse", "--short", "HEAD"]),
    );

    set_env(
        "BUILD_DATE",
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
    );

    // VERSION format: "slim-v1.2.3-<commits_since_tag>-g<commit_hash>"
    // Examples:
    //   - "slim-v1.2.3" (exact tag)
    //   - "slim-v1.2.3-30-g26599324" (30 commits after tag, at commit 26599324...)
    set_env(
        "VERSION",
        Command::new("git").args(["describe", "--tags", "--always", "--match", "slim-v*"]),
    );

    let profile = std::env::var("PROFILE").expect("PROFILE must be set");
    println!("cargo:rustc-env=PROFILE={profile}");

    // Compile protobuf files
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    // export PROTOC to the environment
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    tonic_prost_build::configure()
        .out_dir("src/api/gen")
        .compile_protos(&["proto/v1/data_plane.proto"], &["proto/v1"])
        .unwrap();
}
