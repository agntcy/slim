// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common utilities for slimrpc examples

pub use slim_bindings::{
    RpcAppConfig, RpcAppConnection,
    slimrpc::create_and_connect_app_async as create_local_app,
};

use slim_bindings::slimrpc::error::{Result, SRPCError};
use slim_datapath::messages::Name;

/// Split an ID into its components
/// Expected format: organization/namespace/application
pub fn split_id(id: &str) -> Result<Name> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() < 3 {
        return Err(SRPCError::InvalidId(format!(
            "ID must be in format organization/namespace/app, got: {}",
            id
        )));
    }

    Ok(Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0))
}
