// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0
//
// ConfigLoader loads the configuration file once and exposes lazy, cached
// accessors for tracing, runtime, and services. Services are validated only
// when requested, allowing callers that only need tracing/runtime to proceed
// even if services are absent.

use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_yaml::{Value, from_str};
use thiserror::Error;
use tracing::{debug, warn};

use crate::runtime::RuntimeConfiguration;
use slim_config::component::configuration::Configuration;
use slim_config::component::id::ID;
use slim_config::component::{Component, ComponentBuilder};
use slim_config::provider::ConfigResolver;
use slim_service::{Service, ServiceBuilder};
use slim_tracing::TracingConfiguration;

#[derive(Error, Debug)]
pub enum ConfigError {
    // File / I/O
    #[error("not found: {0}")]
    NotFound(String),

    // Parsing / structural validity
    #[error("invalid configuration - impossible to parse yaml")]
    InvalidYaml,
    #[error("invalid configuration - key {0} not valid")]
    InvalidKey(String),
    #[error("invalid configuration")]
    Invalid(String),

    // YAML decoding (typed propagation)
    #[error("yaml parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    // Services / resolution
    #[error("invalid configuration - missing services")]
    InvalidNoServices,
    #[error("invalid configuration - resolver not found")]
    ResolverError,
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

pub struct ConfigLoader {
    root: Value,
    tracing: Option<TracingConfiguration>,
    runtime: Option<RuntimeConfiguration>,
    services: Option<HashMap<ID, Service>>,
}

impl std::fmt::Debug for ConfigLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tracing_loaded = self.tracing.is_some();
        let runtime_loaded = self.runtime.is_some();
        let services_count = self.services.as_ref().map(|m| m.len());
        let root_keys = self
            .root
            .as_mapping()
            .map(|m| {
                m.keys()
                    .filter_map(|k| k.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        f.debug_struct("ConfigLoader")
            .field("root_keys", &root_keys)
            .field("tracing_loaded", &tracing_loaded)
            .field("runtime_loaded", &runtime_loaded)
            .field("services_count", &services_count)
            .finish()
    }
}

impl ConfigLoader {
    pub fn new(file_path: &str) -> Result<Self, ConfigError> {
        let config_str =
            std::fs::read_to_string(file_path).map_err(|e| ConfigError::NotFound(e.to_string()))?;
        let mut root: Value = from_str(&config_str).map_err(|_| ConfigError::InvalidYaml)?;

        let mapping = root.as_mapping().ok_or(ConfigError::InvalidYaml)?;
        for key in mapping.keys() {
            let k = key.as_str().ok_or(ConfigError::InvalidYaml)?;
            if !CONFIG_KEYS.contains(k) {
                return Err(ConfigError::InvalidKey(k.to_string()));
            }
        }

        let resolver = ConfigResolver::new();
        resolver
            .resolve(&mut root)
            .map_err(|_| ConfigError::ResolverError)?;

        Ok(Self {
            root,
            tracing: None,
            runtime: None,
            services: None,
        })
    }

    pub fn tracing(&mut self) -> &TracingConfiguration {
        if self.tracing.is_none() {
            let cfg = self
                .root
                .get("tracing")
                .cloned()
                .map(|v| {
                    serde_yaml::from_value(v).unwrap_or_else(|e| {
                        warn!(error = ?e, "invalid tracing config, falling back to default");
                        TracingConfiguration::default()
                    })
                })
                .unwrap_or_else(TracingConfiguration::default);
            debug!(?cfg, "Tracing configuration loaded");
            self.tracing = Some(cfg);
        }
        self.tracing.as_ref().unwrap()
    }

    pub fn runtime(&mut self) -> &RuntimeConfiguration {
        if self.runtime.is_none() {
            let cfg = self
                .root
                .get("runtime")
                .cloned()
                .map(|v| {
                    serde_yaml::from_value(v).unwrap_or_else(|e| {
                        warn!(error = ?e, "invalid runtime config, falling back to default");
                        RuntimeConfiguration::default()
                    })
                })
                .unwrap_or_else(RuntimeConfiguration::default);
            debug!(?cfg, "Runtime configuration loaded");
            self.runtime = Some(cfg);
        }
        self.runtime.as_ref().unwrap()
    }

    pub fn services(&mut self) -> Result<&mut HashMap<ID, Service>, ConfigError> {
        if self.services.is_none() {
            let service_val = match self.root.get("services") {
                Some(sv) => sv,
                None => return Err(ConfigError::InvalidNoServices),
            };

            let service_map = service_val.as_mapping().ok_or(ConfigError::InvalidYaml)?;
            debug!(
                count = service_map.len(),
                "Parsing services configuration entries"
            );

            let mut services_ret = HashMap::<ID, Service>::new();
            for (name, value) in service_map {
                let s = build_service(name, value)?;
                services_ret.insert(s.identifier().clone(), s);
            }

            if services_ret.is_empty() {
                return Err(ConfigError::InvalidNoServices);
            }
            let ids: Vec<_> = services_ret.keys().map(|id| id.to_string()).collect();
            debug!(
                count = services_ret.len(),
                ?ids,
                "Services configuration loaded"
            );

            self.services = Some(services_ret);
        }
        Ok(self.services.as_mut().unwrap())
    }
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
    // Typed YAML value conversion (produces ConfigError::YamlError)
    let config: B::Config = serde_yaml::from_value(component_config)?;

    config.validate().map_err(|e| {
        debug!(error = ?e, "Component configuration validation failed");
        ConfigError::Invalid(e.to_string())
    })?;
    debug!(component_id = id.name(), "Resolved component configuration");

    builder
        .build_with_config(id.name(), &config)
        .map_err(|e| ConfigError::Invalid(e.to_string()))
}

fn build_service(name: &Value, config: &Value) -> Result<Service, ConfigError> {
    let id_string = name.as_str().ok_or(ConfigError::InvalidYaml)?;
    let id = ID::new_with_str(id_string).map_err(|e| ConfigError::InvalidKey(e.to_string()))?;
    let component_config = config;

    if id.kind().to_string().as_str() == slim_service::KIND {
        return resolve_component(&id, ServiceBuilder::new(), component_config.clone());
    }

    Err(ConfigError::InvalidKey(id_string.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    fn testdata_path() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/testdata").to_string()
    }

    #[test]
    #[traced_test]
    fn test_full_config_components() {
        let path = format!("{}/config.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");

        let tracing = loader.tracing();
        assert!(
            !tracing.log_level().is_empty(),
            "tracing log level should not be empty"
        );

        let _runtime = loader.runtime();

        let services = loader.services().expect("services should load");
        assert!(!services.is_empty(), "services map should not be empty");
    }

    #[test]
    #[traced_test]
    fn test_missing_services_affects_only_services_loader() {
        let path = format!("{}/config-no-services.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");
        let _ = loader.tracing();
        let _ = loader.runtime();
        let services = loader.services();
        assert!(
            services.is_err(),
            "services loader should error when services are missing"
        );
        matches!(services.unwrap_err(), ConfigError::InvalidNoServices);
    }

    #[test]
    #[traced_test]
    fn test_invalid_yaml() {
        let path = format!("{}/config-invalid-yaml.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).unwrap();

        let res = loader.services();
        assert!(res.is_err(), "services loader should error on invalid yaml");
    }

    #[test]
    #[traced_test]
    fn test_empty_config() {
        let path = format!("{}/config-empty.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).unwrap();

        let res = loader.services();
        assert!(
            res.is_err(),
            "services loader should error when services are missing"
        );
    }

    #[test]
    #[traced_test]
    fn test_tracing_specific_config() {
        let path = format!("{}/config-tracing.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");
        let tracing = loader.tracing();
        assert_eq!(tracing.log_level(), "debug");
    }
}
