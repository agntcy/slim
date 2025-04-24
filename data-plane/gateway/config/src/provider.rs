// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::str;

use serde_yaml::Value;
use thiserror::Error;

mod env;
mod file;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("not found")]
    NotFound,
    #[error("unknown error")]
    Unknown,
}

// Define a trait for reading YAML content from different sources
pub trait ConfigProvider {
    fn load(&self, var: &str) -> Result<String, ProviderError>;
}

#[derive(Default)]
pub struct ConfigResolver {
    providers: HashMap<String, Box<dyn ConfigProvider>>,
}

impl ConfigResolver {
    /// New ConfigResolver
    pub fn new() -> Self {
        let mut providers = HashMap::<String, Box<dyn ConfigProvider>>::new();
        providers.insert("env".to_string(), Box::new(env::EnvConfigProvider));
        providers.insert("file".to_string(), Box::new(file::FileConfigProvider));
        ConfigResolver { providers }
    }

    /// Register a new provider
    pub fn register(&mut self, name: String, provider: Box<dyn ConfigProvider>) {
        self.providers.insert(name, provider);
    }

    /// Resolve given the string
    pub fn resolve_str(&self, value: &str) -> Result<String, ProviderError> {
        let mut value = Value::String(value.to_string());
        self.resolve(&mut value)?;
        Ok(value.as_str().unwrap().to_string())
    }

    /// Resolve the values in the given YAML content
    pub fn resolve(&self, value: &mut Value) -> Result<(), ProviderError> {
        match value {
            Value::String(s) => {
                if let Some(provider_and_ref) =
                    s.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                {
                    let mut parts = provider_and_ref.splitn(2, ':');
                    let provider = parts.next();
                    let var = parts.next();

                    match (provider, var) {
                        (Some(provider), Some(var)) => {
                            if let Some(provider) = self.providers.get(provider) {
                                *s = provider.load(var)?;
                                Ok(())
                            } else {
                                Err(ProviderError::NotFound)
                            }
                        }
                        _ => Err(ProviderError::NotFound),
                    }
                } else {
                    Ok(())
                }
            }
            Value::Sequence(seq) => {
                for item in seq {
                    self.resolve(item)?;
                }

                Ok(())
            }
            Value::Mapping(map) => {
                for (_, v) in map {
                    self.resolve(v)?;
                }

                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;
    use tracing::debug;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_resolve() {
        let testdata_path: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

        debug!("testdata_path: {}", testdata_path);

        // set an env variable to test
        unsafe {
            // ignore clippy warning for this line
            #[allow(clippy::disallowed_methods)]
            std::env::set_var("HOME", "/home/user");
        }

        // Declare a resolver
        let resolver = ConfigResolver::new();

        // Test resolving an env value
        let mut value = Value::String("${env:HOME}".to_string());
        assert!(resolver.resolve(&mut value).is_ok());
        assert!(value.as_str() == Some("/home/user"));

        // Test resolving a file value

        // open file
        let path = format!("{}/testfile", testdata_path);
        let file_str = std::fs::read_to_string(&path).unwrap();

        let mut value = Value::String(format!("${{file:{}}}", path).to_string());
        assert!(resolver.resolve(&mut value).is_ok());
        assert!(value.as_str() == Some(file_str.as_str()));

        // Test resolving a value with an unknown provider
        let mut value = Value::String("${unknown:HOME}".to_string());
        assert!(resolver.resolve(&mut value).is_err());

        // Test resolving a value with an unknown variable
        let mut value = Value::String("${env:UNKNOWN}".to_string());
        assert!(resolver.resolve(&mut value).is_err());

        // Test resolving a value with an invalid format
        let mut value = Value::String("${env:HOME}${env:HOME}".to_string());
        assert!(resolver.resolve(&mut value).is_err());

        // Test resolving a sequence
        let mut value = Value::Sequence(vec![
            Value::String("${env:HOME}".to_string()),
            Value::String(format!("${{file:{}}}", path).to_string()),
        ]);
        assert!(resolver.resolve(&mut value).is_ok());
        assert!(value.as_sequence().unwrap().iter().all(|v| v.is_string()));
        assert!(value[0].as_str().unwrap() == "/home/user");
        assert!(value[1].as_str().unwrap() == file_str);

        // Test resolving a mapping
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            Value::String("env".to_string()),
            Value::String("${env:HOME}".to_string()),
        );
        map.insert(
            Value::String("file".to_string()),
            Value::String(format!("${{file:{}}}", path).to_string()),
        );

        let mut value = Value::Mapping(map);
        assert!(resolver.resolve(&mut value).is_ok());

        let map = value.as_mapping().unwrap();
        assert!(map.iter().all(|(k, v)| k.is_string() && v.is_string()));
        assert!(map[&Value::String("env".to_string())].as_str().unwrap() == "/home/user");
        assert!(map[&Value::String("file".to_string())].as_str().unwrap() == file_str);
    }
}
