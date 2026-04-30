// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    tonic_prost_build::configure()
        .out_dir("src/gen")
        .compile_protos(&["proto/v1/commands.proto"], &["proto/v1"])?;
    Ok(())
}
