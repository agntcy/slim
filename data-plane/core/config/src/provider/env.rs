// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::env;
use std::str;

use super::ConfigProvider;
use super::ProviderError;

// File-based config provider
pub struct EnvConfigProvider;

impl ConfigProvider for EnvConfigProvider {
    fn load(&self, env_var: &str) -> Result<String, ProviderError> {
        env::var(env_var).map_err(|_| ProviderError::NotFound)
    }
}
