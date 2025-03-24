// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::str;

use super::ConfigProvider;
use super::ProviderError;

// File-based config provider
pub struct FileConfigProvider;

impl ConfigProvider for FileConfigProvider {
    fn load(&self, file_path: &str) -> Result<String, ProviderError> {
        fs::read_to_string(file_path).map_err(|_| ProviderError::NotFound)
    }
}
