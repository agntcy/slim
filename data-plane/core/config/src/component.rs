// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod configuration;
pub mod id;

#[async_trait::async_trait]
pub trait Component {
    // Error type for component operations
    type Error: std::error::Error + Send + Sync + 'static;

    // Get name of the component
    fn identifier(&self) -> &id::ID;

    // start the component
    async fn start(&mut self) -> Result<(), Self::Error>;
}

pub trait ComponentBuilder {
    // Associated types
    type Config;
    type Component: Component;

    /// ID of the component
    fn kind(&self) -> id::Kind;

    /// Build the component
    fn build(&self, name: String)
    -> Result<Self::Component, <Self::Component as Component>::Error>;

    /// Build the component with configuration
    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, <Self::Component as Component>::Error>;
}
