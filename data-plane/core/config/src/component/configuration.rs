// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub trait Configuration {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Validate the component configuration
    fn validate(&self) -> Result<(), Self::Error>;
}
