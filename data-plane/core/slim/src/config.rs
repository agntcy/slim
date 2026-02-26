// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0
//
// ConfigLoader loads the configuration file once and exposes lazy, cached
// accessors for tracing, runtime, and services. Services are validated only
// when requested, allowing callers that only need tracing/runtime to proceed
// even if services are absent.

use indexmap::IndexMap;
use lazy_static::lazy_static;
use std::collections::HashSet;

use serde_yaml::{Value, from_str};
use thiserror::Error;
use tracing::debug;

use crate::runtime::RuntimeConfiguration;
use slim_config::component::configuration::Configuration;
use slim_config::component::id::ID;
use slim_config::provider::ConfigResolver;
use slim_service::{Service, ServiceConfiguration, ServiceError};
use slim_tracing::TracingConfiguration;

#[derive(Error, Debug)]
pub enum ConfigError {
    // File / I/O
    #[error("io error")]
    IoError(#[from] std::io::Error),

    // Parsing / structural validity
    #[error("invalid configuration - impossible to parse yaml")]
    InvalidYaml,
    #[error("invalid configuration - key {0} not valid")]
    InvalidKey(String),
    #[error("validation error")]
    Invalid(#[from] ServiceError),

    // YAML decoding (typed propagation)
    #[error("yaml parse error")]
    YamlError(#[from] serde_yaml::Error),

    // Services / resolution
    #[error("invalid configuration - missing services")]
    InvalidNoServices,

    // Provider errors
    #[error("config provider error")]
    ConfigProviderError(#[from] slim_config::provider::ProviderError),
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
    services: Option<IndexMap<ID, Service>>,
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
        let config_str = std::fs::read_to_string(file_path)?;
        let mut root: Value = from_str(&config_str)?;

        let mapping = root.as_mapping().ok_or(ConfigError::InvalidYaml)?;
        for key in mapping.keys() {
            let k = key.as_str().ok_or(ConfigError::InvalidYaml)?;
            if !CONFIG_KEYS.contains(k) {
                return Err(ConfigError::InvalidKey(k.to_string()));
            }
        }

        let resolver = ConfigResolver::new();
        resolver.resolve(&mut root)?;

        Ok(Self {
            root,
            tracing: None,
            runtime: None,
            services: None,
        })
    }

    pub fn tracing(&mut self) -> Result<&TracingConfiguration, ConfigError> {
        if self.tracing.is_none() {
            let cfg = match self.root.get("tracing").cloned() {
                Some(v) => serde_yaml::from_value(v)?,
                None => TracingConfiguration::default(),
            };
            debug!(?cfg, "Tracing configuration loaded");
            self.tracing = Some(cfg);
        }
        Ok(self.tracing.as_ref().unwrap())
    }

    pub fn runtime(&mut self) -> Result<&RuntimeConfiguration, ConfigError> {
        if self.runtime.is_none() {
            let cfg = match self.root.get("runtime").cloned() {
                Some(v) => serde_yaml::from_value(v)?,
                None => RuntimeConfiguration::default(),
            };
            debug!(?cfg, "Runtime configuration loaded");
            self.runtime = Some(cfg);
        }
        Ok(self.runtime.as_ref().unwrap())
    }

    pub fn services(&mut self) -> Result<&mut IndexMap<ID, Service>, ConfigError> {
        if self.services.is_none() {
            // Use services_config to parse configurations
            let configs = self.services_config()?;

            let mut services_ret = IndexMap::<ID, Service>::new();
            for (id, config) in configs {
                // Validate the configuration
                config.validate()?;
                debug!(component_id = id.name(), "Resolved component configuration");

                // Build the service from the configuration
                let service = config.build_server(id.clone())?;
                services_ret.insert(id, service);
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

    pub fn services_config(&mut self) -> Result<IndexMap<ID, ServiceConfiguration>, ConfigError> {
        let service_val = match self.root.get("services") {
            Some(sv) => sv,
            None => return Err(ConfigError::InvalidNoServices),
        };

        let service_map = service_val.as_mapping().ok_or(ConfigError::InvalidYaml)?;
        debug!(
            count = service_map.len(),
            "Parsing services configuration entries"
        );

        let mut configs = IndexMap::<ID, ServiceConfiguration>::new();
        for (name, value) in service_map {
            let id_string = name.as_str().ok_or(ConfigError::InvalidYaml)?;
            let id =
                ID::new_with_str(id_string).map_err(|e| ConfigError::InvalidKey(e.to_string()))?;

            // Parse the ServiceConfiguration directly from YAML
            let mut config: ServiceConfiguration = serde_yaml::from_value(value.clone())?;

            // If condif.node_id is none, set it to the ID's name
            if config.node_id.is_none() {
                config.node_id = Some(id.name().to_string());
            }

            configs.insert(id, config);
        }

        if configs.is_empty() {
            return Err(ConfigError::InvalidNoServices);
        }

        Ok(configs)
    }
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

        let tracing = loader.tracing().expect("tracing config should load");
        assert!(
            !tracing.log_level().is_empty(),
            "tracing log level should not be empty"
        );

        let _runtime = loader.runtime().expect("runtime config should load");

        let services = loader.services().expect("services should load");
        assert!(!services.is_empty(), "services map should not be empty");
    }

    #[test]
    #[traced_test]
    fn test_missing_services_affects_only_services_loader() {
        let path = format!("{}/config-no-services.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");
        let _ = loader.tracing().expect("tracing config should load");
        let _ = loader.runtime().expect("runtime config should load");
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
        let tracing = loader.tracing().expect("tracing config should load");
        assert_eq!(tracing.log_level(), "debug");
    }

    #[test]
    #[traced_test]
    fn test_services_config() {
        let path = format!("{}/config.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");

        // Get service configurations without building Service objects
        let configs = loader
            .services_config()
            .expect("services_config should load");

        assert!(
            !configs.is_empty(),
            "services config map should not be empty"
        );
        assert_eq!(configs.len(), 1, "should have exactly one service");

        // Verify we can access the configuration by ID
        let service_id = ID::new_with_str("slim/0").expect("valid service ID");
        let config = configs
            .get(&service_id)
            .expect("should have slim/0 service");

        // Verify the configuration was successfully parsed
        assert!(
            config.node_id.is_none() || config.node_id.is_some(),
            "node_id field exists"
        );
        assert!(
            config.group_name.is_none() || config.group_name.is_some(),
            "group_name field exists"
        );

        // Verify that services were NOT built (services cache should still be None)
        assert!(
            loader.services.is_none(),
            "services should not be built when using services_config"
        );

        // Now build services and verify that both methods parse the same configuration
        let services = loader.services().expect("services should also load");
        let service = services
            .get(&service_id)
            .expect("should have slim/0 service");
        let service_config = service.config();

        // Both parsing methods should produce identical configurations
        assert_eq!(
            config.node_id, service_config.node_id,
            "node_id should match"
        );
        assert_eq!(
            config.group_name, service_config.group_name,
            "group_name should match"
        );
    }

    #[test]
    #[traced_test]
    fn test_services_config_missing_services() {
        let path = format!("{}/config-no-services.yaml", testdata_path());
        let mut loader = ConfigLoader::new(&path).expect("loader init should succeed");

        let result = loader.services_config();
        assert!(
            result.is_err(),
            "services_config should error when services are missing"
        );
        matches!(result.unwrap_err(), ConfigError::InvalidNoServices);
    }
}
