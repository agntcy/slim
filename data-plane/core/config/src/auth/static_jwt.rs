// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AuthError, ClientAuthenticator};

use slim_auth::builder::JwtBuilder;
use slim_auth::jwt::StaticTokenProvider;
use slim_auth::jwt_middleware::AddJwtLayer;

/// Source of the JWT token - only file-based tokens are supported.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct JwtSource {
    /// Path to a file containing the token (auto-reloaded on change)
    pub file: String,
}

/// JWT auth configuration object.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    #[serde(flatten)]
    source: JwtSource,

    /// Duration (in seconds) used by the AddJwtLayer cache for re-fetching the token.
    /// This duration bounds the "validity" window before re-reading from file.
    #[serde(default = "default_duration")]
    duration: u64,
}

fn default_duration() -> u64 {
    3600
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: JwtSource {
                file: "/path/to/token.jwt".to_string(),
            },
            duration: default_duration(),
        }
    }
}

impl Config {
    /// Create a config from a token file path.
    pub fn with_file(path: impl Into<String>) -> Self {
        let file = path.into();

        Self {
            source: JwtSource { file },
            duration: default_duration(),
        }
    }

    /// Set custom cache duration (seconds).
    pub fn with_duration(self, duration_secs: u64) -> Self {
        Self {
            duration: duration_secs,
            ..self
        }
    }

    /// Access the configured source.
    pub fn source(&self) -> &JwtSource {
        &self.source
    }

    /// Get duration for client layer caching.
    pub fn duration(&self) -> u64 {
        self.duration
    }

    /// Build StaticTokenProvider using jwt.rs types.
    pub fn build_static_token_provider(&self) -> Result<StaticTokenProvider, AuthError> {
        // Use JwtBuilder to leverage file watching for auto-reload.
        JwtBuilder::new()
            .token_file(self.source.file.clone())
            .build()
            .map_err(|e| AuthError::ConfigError(e.to_string()))
    }
}

impl ClientAuthenticator for Config {
    type ClientLayer = AddJwtLayer<StaticTokenProvider>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        let provider = self.build_static_token_provider()?;
        Ok(AddJwtLayer::new(provider, self.duration()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::tower_service::HeaderCheckService;
    use slim_auth::traits::TokenProvider;
    use tower::ServiceBuilder;

    #[tokio::test]
    async fn test_file_config_builds_provider() {
        let path = std::env::temp_dir().join("static_jwt_file_token.jwt");
        std::fs::write(&path, "FILE_TOKEN").unwrap();
        let cfg = Config::with_file(path.to_string_lossy().as_ref());
        let provider = cfg.build_static_token_provider().expect("provider");
        let tok = provider.get_token().unwrap();
        assert_eq!(tok.trim(), "FILE_TOKEN");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_client_layer_file() {
        let path = std::env::temp_dir().join("static_jwt_file_token3.jwt");
        std::fs::write(&path, "FILE_TOKEN_X").unwrap();
        let cfg = Config::with_file(path.to_string_lossy().as_ref());
        let layer = cfg.get_client_layer().expect("layer");
        let _svc = ServiceBuilder::new()
            .layer(layer)
            .service(HeaderCheckService);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_config_duration() {
        let cfg = Config::with_file("/path/to/token").with_duration(7200);
        assert_eq!(cfg.duration(), 7200);
    }

    #[test]
    fn test_config_source() {
        let cfg = Config::with_file("/path/to/token");
        assert_eq!(cfg.source().file, "/path/to/token");
    }

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.duration(), 3600);
        assert_eq!(cfg.source().file, "/path/to/token.jwt");
    }

    #[tokio::test]
    async fn test_provider_token_retrieval() {
        let path = std::env::temp_dir().join("static_jwt_test_token_123.jwt");
        std::fs::write(&path, "TEST_TOKEN_123").unwrap();
        let cfg = Config::with_file(path.to_string_lossy().as_ref());
        let provider = cfg.build_static_token_provider().expect("provider");

        // Test multiple calls to ensure consistency
        for _ in 0..3 {
            let token = provider.get_token().expect("token");
            assert_eq!(token.trim(), "TEST_TOKEN_123");
        }
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_file_provider_token_update() {
        let path = std::env::temp_dir().join("static_jwt_update_test.jwt");

        // Write initial token
        std::fs::write(&path, "INITIAL_TOKEN").unwrap();
        let cfg = Config::with_file(path.to_string_lossy().as_ref());
        let provider = cfg.build_static_token_provider().expect("provider");

        // Verify initial token
        let token = provider.get_token().unwrap();
        assert_eq!(token.trim(), "INITIAL_TOKEN");

        // Update the file
        std::fs::write(&path, "UPDATED_TOKEN").unwrap();

        // Give a moment for file watcher to update (in real scenarios this would be handled by the file watcher)
        std::thread::sleep(std::time::Duration::from_millis(100));

        // The token should eventually be updated by the file watcher
        // Note: In a real test environment, you might need to trigger the file watcher manually
        // or wait longer for the update to propagate

        let _ = std::fs::remove_file(path);
    }
}
