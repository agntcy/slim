// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::error::{Result, SRPCError};
use slim_datapath::messages::Name;

pub const DEADLINE_KEY: &str = "slimrpc-timeout";
pub const MAX_TIMEOUT: u64 = 36000; // 10h in seconds

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

/// Convert a service/method to a subscription Name
pub fn service_and_method_to_name(base_name: &Name, service_method: &str) -> Result<Name> {
    let parts: Vec<&str> = service_method.split('/').collect();
    if parts.len() < 3 {
        return Err(SRPCError::InvalidServiceMethod(
            "Service method must be in format /service/method".to_string(),
        ));
    }

    let service_name = parts[1];
    let method_name = parts[2];

    method_to_name(base_name, service_name, method_name)
}

/// Convert service and method names to a subscription Name
pub fn method_to_name(base_name: &Name, service_name: &str, method_name: &str) -> Result<Name> {
    let components = base_name.to_string();
    let parts: Vec<&str> = components.split('/').collect();

    if parts.len() < 3 {
        return Err(SRPCError::InvalidBaseName(
            "Base name must have at least 3 components".to_string(),
        ));
    }

    let subscription_name = format!("{}-{}-{}", parts[2], service_name, method_name);

    Ok(Name::from_strings([parts[0], parts[1], &subscription_name]))
}
