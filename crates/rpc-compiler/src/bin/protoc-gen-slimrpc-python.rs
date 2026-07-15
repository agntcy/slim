// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agntcy_protoc_slimrpc_plugin::common;
use agntcy_protoc_slimrpc_plugin::python;
use anyhow::Result;

fn main() -> Result<()> {
    let request = common::read_request()?;
    let response = python::generate(request)?;
    common::write_response(&response)?;
    Ok(())
}
