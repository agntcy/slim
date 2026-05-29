// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use agntcy_protoc_slimrpc_plugin::common;
use agntcy_protoc_slimrpc_plugin::csharp;
use anyhow::Result;

fn main() -> Result<()> {
    let request = common::read_request()?;
    let response = csharp::generate(request)?;
    common::write_response(&response)?;
    Ok(())
}
