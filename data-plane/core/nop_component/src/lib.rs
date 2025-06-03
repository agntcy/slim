// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

use slim_config::component::configuration::{Configuration, ConfigurationError};
use slim_config::component::id::{ID, Kind};
use slim_config::component::{Component, ComponentBuilder, ComponentError};
use slim_config::grpc::client::ClientConfig;

// Define the kind of the component as static string
pub const KIND: &str = "nop_component";

#[derive(Debug, Default, Clone, Deserialize)]
pub struct NopComponentConfig {
    // Component-specific fields
    #[serde(default)]
    pub grpc: ClientConfig,

    // other fields (mandatory)
    pub field: String,
}

// Define a nop component that implements the trait
#[derive(Debug)]
pub struct NopComponent {
    // ID of the component
    id: ID,

    // Configuration field
    #[allow(dead_code)]
    config: NopComponentConfig,
}

impl NopComponent {
    // Create a new NopComponentConfig
    pub fn new(id: ID) -> Self {
        NopComponent {
            id,
            config: NopComponentConfig::default(),
        }
    }

    // Create a new NopComponentConfig with configuration
    pub fn new_with_config(id: ID, config: &NopComponentConfig) -> Self {
        let config = config.clone();

        NopComponent { id, config }
    }
}

impl Configuration for NopComponentConfig {
    // Validate the component configuration
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate the component configuration
        Ok(())
    }
}

// Implement the ConfigurableComponent trait for NopComponent
impl Component for NopComponent {
    // Get the id of the component
    fn identifier(&self) -> &ID {
        &self.id
    }

    // Start the component
    async fn start(&mut self) -> Result<(), ComponentError> {
        // Start the component
        Ok(())
    }
}

/// Implement the ComponentBuilder trait for NopComponent
#[derive(PartialEq, Eq, Hash, Default)]
pub struct NopComponentBuilder;

impl NopComponentBuilder {
    // Create a new NopComponentBuilder
    pub fn new() -> Self {
        NopComponentBuilder {}
    }

    pub fn kind() -> Kind {
        Kind::new(KIND).unwrap()
    }
}

impl ComponentBuilder for NopComponentBuilder {
    type Config = NopComponentConfig;
    type Component = NopComponent;

    // ID of the component
    fn kind(&self) -> Kind {
        NopComponentBuilder::kind()
    }

    // Build the component
    fn build(&self, name: String) -> Result<Self::Component, ComponentError> {
        let id = ID::new_with_name(NopComponentBuilder::kind(), name.as_ref())
            .map_err(|e| ComponentError::ConfigError(e.to_string()))?;

        Ok(NopComponent::new(id))
    }

    // Build the component
    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, ComponentError> {
        let id = ID::new_with_name(NopComponentBuilder::kind(), name)
            .map_err(|e| ComponentError::ConfigError(e.to_string()))?;

        Ok(NopComponent::new_with_config(id, config))
    }
}
