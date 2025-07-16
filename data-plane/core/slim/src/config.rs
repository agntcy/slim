// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_yaml::{Value, from_str};
use thiserror::Error;
use tracing::debug;

use crate::runtime::RuntimeConfiguration;
use slim_config::component::configuration::Configuration;
use slim_config::component::id::ID;
use slim_config::component::{Component, ComponentBuilder};
use slim_config::provider::ConfigResolver;
use slim_service::{Service, ServiceBuilder};
use slim_tracing::TracingConfiguration;

pub struct ConfigResult {
    /// tracing configuration
    #[allow(dead_code)]
    pub tracing: TracingConfiguration,

    /// runtime configuration
    pub runtime: RuntimeConfiguration,

    /// map of services
    pub services: HashMap<ID, Service>,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid configuration - impossible to parse yaml")]
    InvalidYaml,
    #[error("invalid configuration - key {0} not valid")]
    InvalidKey(String),
    #[error("invalid configuration - missing services")]
    InvalidNoServices,
    #[error("invalid configuration - resolver not found")]
    ResolverError,
    #[error("invalid configuration")]
    Invalid(String),
}

lazy_static! {
    static ref CONFIG_KEYS: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("tracing");
        s.insert("runtime");
        s.insert("services");
        s
    };
}

fn resolve_component<B>(
    id: &ID,
    builder: B,
    component_config: Value,
) -> Result<B::Component, ConfigError>
where
    B: ComponentBuilder,
    B::Config: Configuration + std::fmt::Debug,
    for<'de> <B as ComponentBuilder>::Config: Deserialize<'de>,
{
    let config: B::Config = serde_yaml::from_value(component_config)
        .map_err(|e| ConfigError::Invalid(e.to_string()))?;

    // Validate the configuration
    config
        .validate()
        .map_err(|e| ConfigError::Invalid(e.to_string()))?;

    // record the configuration
    debug!(?config);

    builder
        .build_with_config(id.name(), &config)
        .map_err(|e| ConfigError::Invalid(e.to_string()))
}

fn build_service(name: &Value, config: &Value) -> Result<Service, ConfigError> {
    let id_string = name.as_str().unwrap();
    let id = ID::new_with_str(id_string).map_err(|e| ConfigError::InvalidKey(e.to_string()))?;
    let component_config = config;

    if id.kind().to_string().as_str() == slim_service::KIND {
        return resolve_component(&id, ServiceBuilder::new(), component_config.clone());
    }

    Err(ConfigError::InvalidKey(id_string.to_string()))
}

pub fn load_config(config_file: &str) -> Result<ConfigResult, ConfigError> {
    let config_str =
        std::fs::read_to_string(config_file).map_err(|e| ConfigError::NotFound(e.to_string()))?;
    let mut config: Value = from_str(&config_str).map_err(|_| ConfigError::InvalidYaml)?;

    // Validate fields in configuration
    for key in config.as_mapping().unwrap().keys() {
        let k = key.as_str().unwrap();
        if !CONFIG_KEYS.contains(k) {
            return Err(ConfigError::InvalidKey(k.to_string()));
        }
    }

    // Resolve the configuration
    let resolver = ConfigResolver::new();
    resolver
        .resolve(&mut config)
        .map_err(|_| ConfigError::ResolverError)?;

    // Configure tracing
    let tracing_config = config.get("tracing");
    let tracing = match tracing_config {
        Some(tracing) => serde_yaml::from_value(tracing.clone())
            .map_err(|e| ConfigError::Invalid(e.to_string()))?,
        None => TracingConfiguration::default(),
    };

    // configure runtime
    let runtime = match config.get("runtime") {
        Some(runtime) => serde_yaml::from_value(runtime.clone())
            .map_err(|e| ConfigError::Invalid(e.to_string()))?,
        None => RuntimeConfiguration::default(),
    };

    // log the runtime configuration
    debug!(?runtime);

    // Configure services
    let service = match config.get("services") {
        Some(service) => match service.as_mapping() {
            Some(service) => service,
            None => return Err(ConfigError::InvalidYaml),
        },
        None => return Err(ConfigError::InvalidNoServices),
    };
    let mut services_ret = HashMap::<ID, Service>::new();
    for (name, value) in service {
        let s = build_service(name, value)?;
        services_ret.insert(s.identifier().clone(), s);
    }
    if services_ret.is_empty() {
        return Err(ConfigError::InvalidNoServices);
    }

    Ok(ConfigResult {
        tracing,
        runtime,
        services: services_ret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_load_config() {
        let testdata_path: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

        debug!("Testdata path: {}", testdata_path);

        let mut config_path = format!("{}/config.yaml", testdata_path);
        let mut config = load_config(config_path.as_str());
        assert!(config.is_ok(), "error: {:?}", config.err());

        config_path = format!("{}/config-empty.yaml", testdata_path);
        config = load_config(config_path.as_str());
        assert!(config.is_err());

        config_path = format!("{}/config-no-services.yaml", testdata_path);
        config = load_config(config_path.as_str());
        assert!(config.is_err());

        config_path = format!("{}/config-invalid-yaml.yaml", testdata_path);
        config = load_config(config_path.as_str());
        assert!(config.is_err());

        config_path = format!("{}/config-tracing.yaml", testdata_path);
        config = load_config(config_path.as_str());
        assert!(config.is_ok(), "error: {:?}", config.err());
        assert!(config.unwrap().tracing.log_level() == "debug");
    }
}
