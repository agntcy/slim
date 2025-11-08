// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;

use crate::bindings::adapter::BindingsAdapter;
use crate::errors::ServiceError;
use crate::service::Service;

/// Builder for creating BindingsAdapter instances
pub struct AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    app_name: Option<Name>,
    identity_provider: Option<P>,
    identity_verifier: Option<V>,
}

impl<P, V> AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            app_name: None,
            identity_provider: None,
            identity_verifier: None,
        }
    }

    /// Set the app name
    pub fn with_name(mut self, name: Name) -> Self {
        self.app_name = Some(name);
        self
    }

    /// Set the identity provider
    pub fn with_identity_provider(mut self, provider: P) -> Self {
        self.identity_provider = Some(provider);
        self
    }

    /// Set the identity verifier
    pub fn with_identity_verifier(mut self, verifier: V) -> Self {
        self.identity_verifier = Some(verifier);
        self
    }

    /// Build the AppAdapter using a Service instance
    pub fn build(self, service: &Service) -> Result<BindingsAdapter<P, V>, ServiceError> {
        let app_name = self
            .app_name
            .ok_or_else(|| ServiceError::ConfigError("app name is required".to_string()))?;

        let identity_provider = self.identity_provider.ok_or_else(|| {
            ServiceError::ConfigError("identity provider is required".to_string())
        })?;

        let identity_verifier = self.identity_verifier.ok_or_else(|| {
            ServiceError::ConfigError("identity verifier is required".to_string())
        })?;

        // Use Service to create the App and get the notification receiver
        let (app, rx_app) = service.create_app(&app_name, identity_provider, identity_verifier)?;

        Ok(BindingsAdapter::new_with_app(app, rx_app))
    }
}

impl<P, V> Default for AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::messages::Name;
    use slim_testing::utils::TEST_VALID_SECRET;

    use slim_config::component::ComponentBuilder;

    type TestProvider = SharedSecret;
    type TestVerifier = SharedSecret;

    /// Create a mock service for testing
    async fn create_test_service() -> Service {
        Service::builder()
            .build("test-service".to_string())
            .expect("Failed to create test service")
    }

    /// Create test authentication components
    fn create_test_auth() -> (TestProvider, TestVerifier) {
        let provider = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let verifier = SharedSecret::new("test-app", TEST_VALID_SECRET);
        (provider, verifier)
    }

    /// Create test app name
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }

    #[tokio::test]
    async fn test_builder_pattern() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::builder()
            .with_name(app_name)
            .with_identity_provider(provider)
            .with_identity_verifier(verifier)
            .build(&service)
            .expect("Builder should work with all required fields");

        assert!(adapter.id() > 0);
        assert_eq!(
            adapter.name().components_strings(),
            &["org", "namespace", "test-app"]
        );
    }

    #[tokio::test]
    async fn test_builder_missing_name() {
        let service = create_test_service().await;
        let (provider, verifier) = create_test_auth();

        let result = BindingsAdapter::builder()
            .with_identity_provider(provider)
            .with_identity_verifier(verifier)
            .build(&service);

        assert!(result.is_err());
        let error_string = match result {
            Err(e) => e.to_string(),
            _ => panic!("Expected an error"),
        };
        assert!(error_string.contains("app name is required"));
    }

    #[tokio::test]
    async fn test_builder_missing_provider() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (_, verifier) = create_test_auth();

        let result = BindingsAdapter::<TestProvider, TestVerifier>::builder()
            .with_name(app_name)
            .with_identity_verifier(verifier)
            .build(&service);

        assert!(result.is_err());
        let error_string = match result {
            Err(e) => e.to_string(),
            _ => panic!("Expected an error"),
        };
        assert!(error_string.contains("identity provider is required"));
    }

    #[tokio::test]
    async fn test_builder_missing_verifier() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, _) = create_test_auth();

        let result = BindingsAdapter::<TestProvider, TestVerifier>::builder()
            .with_name(app_name)
            .with_identity_provider(provider)
            .build(&service);

        assert!(result.is_err());
        let error_string = match result {
            Err(e) => e.to_string(),
            _ => panic!("Expected an error"),
        };
        assert!(error_string.contains("identity verifier is required"));
    }

    #[tokio::test]
    async fn test_builder_default() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = AppAdapterBuilder::<TestProvider, TestVerifier>::default()
            .with_name(app_name)
            .with_identity_provider(provider)
            .with_identity_verifier(verifier)
            .build(&service)
            .expect("Default builder should work");

        assert!(adapter.id() > 0);
        assert_eq!(
            adapter.name().components_strings(),
            &["org", "namespace", "test-app"]
        );
    }
}
