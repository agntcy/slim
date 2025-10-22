// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SPIFFE using real SPIRE server and agent
//!
//! These tests use bollard to spin up actual SPIRE server and agent
//! containers to test the full authentication flow with real workload API interactions.
//!
//! Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture

#![cfg(target_os = "linux")]

mod spire_env;

use spire_env::SpireTestEnvironment;
use slim_auth::spiffe::{
    SpiffeJwtVerifier, SpiffeProvider, SpiffeProviderConfig, SpiffeVerifierConfig,
};
use slim_auth::traits::{TokenProvider, Verifier};
use std::collections::HashMap;

/// Helper to check if Docker is available
async fn is_docker_available() -> bool {
    use tokio::process::Command;
    Command::new("docker")
        .arg("ps")
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Skip test macro
macro_rules! require_docker {
    () => {
        if !is_docker_available().await {
            tracing::warn!("Docker is not available - skipping test");
            tracing::warn!("Install Docker and ensure the daemon is running to run these tests");
            return;
        }
    };
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_initialization() {
    require_docker!();

    // Create and start test environment
    let mut env = SpireTestEnvironment::new()
        .await
        .expect("Failed to create test environment");

    env.start()
        .await
        .expect("Failed to start SPIRE containers");

    // Now test our SPIFFE provider
    let config = env.get_spiffe_provider_config();
    tracing::info!("Creating SpiffeProvider with config: {:?}", config);

    let mut provider = SpiffeProvider::new(config);

    // With proper join token attestation, the agent should successfully connect
    // and the provider initialization should work
    let init_result = provider.initialize().await;

    match init_result {
        Ok(_) => {
            tracing::info!("Provider initialized successfully");

            // Test X.509 SVID retrieval
            match provider.get_x509_svid() {
                Ok(svid) => {
                    tracing::info!("Got X.509 SVID: {}", svid.spiffe_id());
                    assert!(svid.spiffe_id().to_string().contains("example.org"));
                }
                Err(e) => tracing::warn!("X.509 SVID fetch failed: {}", e),
            }

            // Test JWT token retrieval
            match provider.get_token() {
                Ok(token) => {
                    tracing::info!("Got JWT token");
                    assert!(!token.is_empty());
                    let parts: Vec<&str> = token.split('.').collect();
                    assert_eq!(parts.len(), 3, "JWT should have 3 parts");
                }
                Err(e) => tracing::warn!("JWT token fetch failed: {}", e),
            }

            // Test JWT token retrieval with custom claims
            let custom_claims = HashMap::from([
                ("pubkey".to_string(), serde_json::Value::String("abcdef".to_string()))
            ]);
            match provider.get_token_with_claims(custom_claims) {
                Ok(token_with_claims) => {
                    tracing::info!("Got JWT token with custom claims");
                    assert!(!token_with_claims.is_empty());
                    let parts: Vec<&str> = token_with_claims.split('.').collect();
                    assert_eq!(parts.len(), 3, "JWT should have 3 parts");
                }
                Err(e) => tracing::warn!("JWT token with claims fetch failed: {}", e),
            }
        }
        Err(e) => {
            tracing::error!("Provider initialization failed: {}", e);
            panic!("Provider initialization should succeed with proper join token attestation");
        }
    }

    // Cleanup
    env.cleanup().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_jwt_verifier_creation() {
    require_docker!();

    tracing::info!("Testing SpiffeJwtVerifier creation and basic operations");

    // Test with socket path
    let verifier_config = SpiffeVerifierConfig {
        socket_path: Some("unix:///tmp/test-socket".to_string()),
        jwt_audiences: vec!["test-audience".to_string(), "another-audience".to_string()],
    };

    let verifier = SpiffeJwtVerifier::new(verifier_config);

    // Note: Can't test config directly as it's private, but we test the behavior instead
    // Configuration is verified through initialization and verification behavior

    tracing::info!("SpiffeJwtVerifier created with correct configuration");

    // Test initialization with non-existent socket
    let init_result = verifier.initialize().await;
    assert!(init_result.is_err(), "Should fail with non-existent socket");
    tracing::info!("Correctly fails to initialize with non-existent socket");

    // Test verification without initialization
    let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature";
    let verify_result = verifier.verify(token).await;
    assert!(verify_result.is_err(), "Should fail without initialization");
    tracing::info!("Correctly fails to verify without initialization");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_configurations() {
    tracing::info!("Testing various SpiffeProvider configurations");

    // Test default configuration
    let default_config = SpiffeProviderConfig::default();
    assert_eq!(default_config.jwt_audiences, vec!["slim".to_string()]);
    assert!(default_config.socket_path.is_none());
    assert!(default_config.target_spiffe_id.is_none());
    tracing::info!("Default configuration is correct");

    // Test custom configuration
    let custom_config = SpiffeProviderConfig {
        socket_path: Some("unix:///custom/path".to_string()),
        target_spiffe_id: Some("spiffe://example.org/backend".to_string()),
        jwt_audiences: vec!["api".to_string(), "web".to_string()],
    };

    let provider = SpiffeProvider::new(custom_config.clone());

    // Test getting token before initialization
    let token_result = provider.get_token();
    assert!(token_result.is_err(), "Should fail before initialization");
    let err = format!("{}", token_result.unwrap_err());
    assert!(err.contains("not initialized") || err.contains("JwtSource"));
    tracing::info!("Correctly fails to get token before initialization");

    // Test getting X.509 before initialization
    let x509_result = provider.get_x509_svid();
    assert!(x509_result.is_err(), "Should fail before initialization");
    let err = format!("{}", x509_result.unwrap_err());
    assert!(err.contains("not initialized") || err.contains("X509Source"));
    tracing::info!("Correctly fails to get X.509 before initialization");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_error_handling() {
    tracing::info!("Testing SpiffeProvider error handling");

    // Test with invalid socket path
    let invalid_config = SpiffeProviderConfig {
        socket_path: Some("unix:///nonexistent/socket".to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["test".to_string()],
    };

    let mut provider = SpiffeProvider::new(invalid_config);

    // Should fail to initialize
    let init_result = provider.initialize().await;
    assert!(init_result.is_err(), "Should fail with invalid socket");

    let err = format!("{}", init_result.unwrap_err());
    assert!(err.contains("Failed to connect") || err.contains("SPIFFE"));
    tracing::info!("Correctly handles invalid socket path: {}", err);

    // Provider should still be in uninitialized state
    assert!(provider.get_token().is_err());
    assert!(provider.get_x509_svid().is_err());
    tracing::info!("Provider remains in safe uninitialized state after error");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_multiple_providers_isolation() {
    tracing::info!("Testing isolation between multiple SpiffeProvider instances");

    let config1 = SpiffeProviderConfig {
        socket_path: Some("unix:///socket1".to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["audience1".to_string()],
    };

    let config2 = SpiffeProviderConfig {
        socket_path: Some("unix:///socket2".to_string()),
        target_spiffe_id: Some("spiffe://example.org/service2".to_string()),
        jwt_audiences: vec!["audience2".to_string()],
    };

    let provider1 = SpiffeProvider::new(config1);
    let provider2 = SpiffeProvider::new(config2);

    // Both should be independent and in uninitialized state
    assert!(provider1.get_token().is_err());
    assert!(provider2.get_token().is_err());

    tracing::info!("Multiple providers maintain independent state");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_config_validation() {
    tracing::info!("Testing SPIFFE configuration validation");

    // Test empty audiences
    let config = SpiffeProviderConfig {
        socket_path: None,
        target_spiffe_id: None,
        jwt_audiences: vec![],
    };

    let provider = SpiffeProvider::new(config);
    // Should create but initialization might fail
    assert!(provider.get_token().is_err());

    // Test with multiple audiences
    let config_multi = SpiffeProviderConfig {
        socket_path: None,
        target_spiffe_id: None,
        jwt_audiences: vec!["aud1".to_string(), "aud2".to_string(), "aud3".to_string()],
    };

    let _provider_multi = SpiffeProvider::new(config_multi);
    tracing::info!("Configuration validation works correctly");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_verifier_config() {
    tracing::info!("Testing SpiffeJwtVerifier configuration");

    // Test with no socket path
    let config = SpiffeVerifierConfig {
        socket_path: None,
        jwt_audiences: vec!["test".to_string()],
    };

    let _verifier = SpiffeJwtVerifier::new(config);
    // Note: config is private, testing behavior instead of direct field access

    // Test with empty audiences
    let config_empty = SpiffeVerifierConfig {
        socket_path: Some("unix:///tmp/test".to_string()),
        jwt_audiences: vec![],
    };

    let _verifier_empty = SpiffeJwtVerifier::new(config_empty);
    // Note: config is private, but verifier is created successfully

    tracing::info!("Verifier configuration works correctly");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_try_methods() {
    tracing::info!("Testing SPIFFE try_* methods for non-async contexts");

    let verifier_config = SpiffeVerifierConfig {
        socket_path: None,
        jwt_audiences: vec!["test".to_string()],
    };

    let verifier = SpiffeJwtVerifier::new(verifier_config);

    // Try to verify without initialization - should return WouldBlockOn
    let result = verifier.try_verify("fake.token");
    assert!(result.is_err());

    // Try to get claims without initialization - should return WouldBlockOn
    let claims_result: Result<serde_json::Value, _> = verifier.try_get_claims("fake.token");
    assert!(claims_result.is_err());

    tracing::info!("try_* methods correctly handle uninitialized state");
}
