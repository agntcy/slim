// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .out_dir("src/gen")
        .compile_protos(&["proto/v1/commands.proto"], &["proto/v1"])?;
    Ok(())
}
