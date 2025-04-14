// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

pub mod configuration;
pub mod id;

// Define the trait for the base component
#[derive(Error, Debug)]
pub enum ComponentError {
    #[error("configuration error {0}")]
    ConfigError(String),
    #[error("runtime error: {0}")]
    RuntimeError(String),
    #[error("unknown error")]
    Unknown,
}

pub trait Component {
    // Get name of the component
    fn identifier(&self) -> &id::ID;

    // start the component
    #[allow(async_fn_in_trait)]
    async fn start(&mut self) -> Result<(), ComponentError>;
}

pub trait ComponentBuilder {
    // Associated types
    type Config;
    type Component: Component;

    /// ID of the component
    fn kind(&self) -> id::Kind;

    /// Build the component
    fn build(&self, name: String) -> Result<Self::Component, ComponentError>;

    /// Build the component with configuration
    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, ComponentError>;
}
