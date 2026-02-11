// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agntcy_protoc_slimrpc_plugin::common;
use agntcy_protoc_slimrpc_plugin::golang;
use agntcy_protoc_slimrpc_plugin::python;
use anyhow::{anyhow, Result};
use std::env;
use std::path::Path;

fn main() -> Result<()> {
    let request = common::read_request()?;

    // Determine which plugin to use based on the executable name
    let exe_name = env::args()
        .next()
        .and_then(|arg0| {
            Path::new(&arg0)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| anyhow!("Could not determine executable name"))?;

    let response = if exe_name.contains("python") {
        python::generate(request)?
    } else if exe_name.contains("go") {
        golang::generate(request)?
    } else {
        return Err(anyhow!(
            "Unknown plugin type. Executable name must contain 'python' or 'go', got: {}",
            exe_name
        ));
    };

    common::write_response(&response)?;
    Ok(())
}
