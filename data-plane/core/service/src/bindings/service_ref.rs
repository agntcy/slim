// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::OnceLock;

use crate::service::Service;
use slim_config::component::ComponentBuilder;

// Global static service instance for bindings
static GLOBAL_SERVICE: OnceLock<Service> = OnceLock::new();

/// Service reference type for bindings
pub enum ServiceRef {
    Global(&'static Service),
    Local(Box<Service>),
}

impl ServiceRef {
    /// Get the service reference
    pub fn get_service(&self) -> &Service {
        match self {
            ServiceRef::Global(s) => s,
            ServiceRef::Local(s) => s,
        }
    }
}

/// Get or initialize the global service for bindings
pub fn get_or_init_global_service() -> &'static Service {
    GLOBAL_SERVICE.get_or_init(|| {
        Service::builder()
            .build("global-bindings-service".to_string())
            .expect("Failed to create global bindings service")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::messages::Name;
    use slim_testing::utils::TEST_VALID_SECRET;

    use crate::bindings::adapter::BindingsAdapter;

    /// Create test authentication components
    fn create_test_auth() -> (AuthProvider, AuthVerifier) {
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);
        (provider, verifier)
    }

    /// Create test app name
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }

    #[tokio::test]
    async fn test_global_service_singleton() {
        let service1 = get_or_init_global_service();
        let service2 = get_or_init_global_service();

        // They should be the same instance (same memory address)
        assert!(std::ptr::eq(service1, service2));
    }

    #[tokio::test]
    async fn test_service_ref_get_service() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get global service instance
        let _global_service = get_or_init_global_service();

        // Test local service ref - just ensure it creates without error
        let _local_adapter =
            BindingsAdapter::new(base_name.clone(), provider.clone(), verifier.clone(), true)
                .unwrap();
        // Note: ServiceRef is now private to the adapter, can't directly test pointer equality

        // Test global service ref - just ensure it creates without error
        let _global_adapter = BindingsAdapter::new(base_name, provider, verifier, false).unwrap();
        // Note: ServiceRef is now private to the adapter, can't directly test pointer equality
    }
}
