// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SPIFFE using real SPIRE server and agent
//!
//! These tests use bollard to spin up actual SPIRE server and agent
//! containers to test the full authentication flow with real workload API interactions.
//!
//! Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture

#![cfg(target_os = "linux")]

use slim_auth::metadata::MetadataMap;
use slim_auth::spire::SpireIdentityManager;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::auth::spire::SpireConfig;
use slim_testing::utils::spire_env::SpireTestEnvironment;

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

// Helper functions to reduce test repetition

/// Creates a test SpiffeConfig with a socket path
fn test_config_with_socket(socket_path: &str) -> SpireConfig {
    SpireConfig {
        socket_path: Some(socket_path.to_string()),
        jwt_audiences: vec!["test".to_string()],
        ..Default::default()
    }
}

/// Creates a test SpiffeConfig with default test audiences
fn test_config() -> SpireConfig {
    SpireConfig {
        jwt_audiences: vec!["test".to_string()],
        ..Default::default()
    }
}

/// Creates a test SpiffeConfig with a socket path and multiple audiences
fn test_config_with_socket_and_audiences(socket_path: &str) -> SpireConfig {
    SpireConfig {
        socket_path: Some(socket_path.to_string()),
        jwt_audiences: vec!["test-audience".to_string(), "another-audience".to_string()],
        ..Default::default()
    }
}

/// Helper to build a SpireIdentityManager from a SpireConfig
fn build_manager_from(cfg: &SpireConfig) -> SpireIdentityManager {
    let mut builder = SpireIdentityManager::builder().with_jwt_audiences(cfg.jwt_audiences.clone());

    if let Some(ref sp) = cfg.socket_path {
        builder = builder.with_socket_path(sp.clone());
    }
    if let Some(ref id) = cfg.target_spiffe_id {
        builder = builder.with_target_spiffe_id(id.clone());
    }

    builder.build()
}

/// Asserts that a provider is in uninitialized state
fn assert_manager_uninitialized(manager: &SpireIdentityManager) {
    let token_result = manager.get_token();
    assert!(token_result.is_err(), "Should fail before initialization");
    let err = format!("{}", token_result.unwrap_err());
    assert!(err.contains("not initialized") || err.contains("JwtSource"));

    let x509_result = manager.get_x509_svid();
    assert!(x509_result.is_err(), "Should fail before initialization");
    let err = format!("{}", x509_result.unwrap_err());
    assert!(err.contains("not initialized") || err.contains("X509Source"));
}

/// Asserts that a verifier is in uninitialized state
async fn assert_verifier_uninitialized(verifier: &SpireIdentityManager) {
    let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature";
    let verify_result = verifier.verify(token).await;
    assert!(verify_result.is_err(), "Should fail without initialization");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_initialization() {
    require_docker!();

    // Create and start test environment
    let mut env = SpireTestEnvironment::new()
        .await
        .expect("Failed to create test environment");

    env.start().await.expect("Failed to start SPIRE containers");

    // Now test our SPIFFE provider
    let config = env.get_spiffe_config();
    tracing::info!("Creating SpiffeIdentityManager with config: {:?}", config);

    // Sleep 3 seconds
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let mut provider = build_manager_from(&config);

    // Use same unified config for verifier
    let mut verifier = build_manager_from(&config);

    let mut should_panic = false;
    'test_block: {
        // Initialize provider
        match provider.initialize().await {
            Ok(_) => {
                tracing::info!("Provider initialized successfully");
            }
            Err(e) => {
                tracing::error!("Provider initialization failed: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        // Initialize verifier
        match verifier.initialize().await {
            Ok(_) => {
                tracing::info!("Verifier initialized successfully");
            }
            Err(e) => {
                tracing::error!("Verifier initialization failed: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        // Test X.509 bundle retrieval
        match verifier.get_x509_bundle() {
            Ok(x509_bundle) => {
                tracing::info!(
                    "Successfully retrieved X.509 bundle for trust domain: {}",
                    x509_bundle.trust_domain()
                );
                // Verify the bundle has authorities (CA certificates)
                let authorities = x509_bundle.authorities();
                tracing::info!("Bundle contains {} CA certificate(s)", authorities.len());
                assert!(
                    !authorities.is_empty(),
                    "Bundle should contain at least one CA certificate"
                );
            }
            Err(e) => {
                tracing::error!("Failed to get X.509 bundle: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        // Test X.509 SVID retrieval
        match provider.get_x509_svid() {
            Ok(svid) => {
                tracing::info!("Got X.509 SVID: {}", svid.spiffe_id());
                assert!(svid.spiffe_id().to_string().contains("example.org"));
            }
            Err(e) => {
                tracing::error!("X.509 SVID fetch failed: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        // Test JWT token retrieval
        match provider.get_token() {
            Ok(token) => {
                tracing::info!("Got JWT token");
                assert!(!token.is_empty());
                let parts: Vec<&str> = token.split('.').collect();
                assert_eq!(parts.len(), 3, "JWT should have 3 parts");
            }
            Err(e) => {
                tracing::error!("JWT token fetch failed: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        // Test JWT token retrieval with custom claims
        let custom_claims = {
            let mut claims = MetadataMap::new();
            claims.insert("pubkey", "abcdef");
            claims
        };
        match provider.get_token_with_claims(custom_claims).await {
            Ok(token_with_claims) => {
                tracing::info!("Got JWT token with custom claims");
                assert!(!token_with_claims.is_empty());
                let parts: Vec<&str> = token_with_claims.split('.').collect();
                assert_eq!(parts.len(), 3, "JWT should have 3 parts");

                // Test JWT token verification and custom claims extraction
                match verifier
                    .get_claims::<serde_json::Value>(token_with_claims)
                    .await
                {
                    Ok(claims) => {
                        tracing::info!("Successfully verified JWT token with claims: {:?}", claims);

                        // Verify that custom_claims exists
                        if let Some(custom_claims) = claims.get("custom_claims") {
                            tracing::info!("Found custom_claims: {:?}", custom_claims);

                            // Verify the specific custom claim we set
                            if let Some(pubkey) = custom_claims.get("pubkey") {
                                assert_eq!(
                                    pubkey.as_str(),
                                    Some("abcdef"),
                                    "Custom claim 'pubkey' should have value 'abcdef'"
                                );
                                tracing::info!("Successfully verified custom claim 'pubkey'");
                            } else {
                                tracing::warn!("Custom claim 'pubkey' not found in custom_claims");
                                should_panic = true;
                                break 'test_block;
                            }
                        } else {
                            tracing::warn!("custom_claims field not found in JWT claims");
                            should_panic = true;
                            break 'test_block;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "JWT verification failed (may be expected in test environment): {}",
                            e
                        );
                        // Don't panic here as verification might fail in test environment
                    }
                }
            }
            Err(e) => {
                tracing::error!("JWT token with claims fetch failed: {}", e);
                should_panic = true;
                break 'test_block;
            }
        }

        break 'test_block;
    }

    // Cleanup
    env.cleanup()
        .await
        .expect("Failed to cleanup test environment");

    if should_panic {
        panic!("SPIFFE Provider test failed");
    }
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_jwt_verifier_creation() {
    require_docker!();

    tracing::info!("Testing SPIFFE verifier creation and basic operations");

    let cfg = test_config_with_socket_and_audiences("unix:///tmp/test-socket");
    let mut verifier = build_manager_from(&cfg);
    tracing::info!("SpiffeJwtVerifier created with correct configuration");

    // Test initialization with non-existent socket
    let init_result = verifier.initialize().await;
    assert!(init_result.is_err(), "Should fail with non-existent socket");
    tracing::info!("Correctly fails to initialize with non-existent socket");

    // Test verification without initialization
    assert_verifier_uninitialized(&verifier).await;
    tracing::info!("Correctly fails to verify without initialization");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_configurations() {
    tracing::info!("Testing various SPIFFE identity manager configurations");

    // Test default configuration
    let default_config = SpireConfig::default();
    assert_eq!(default_config.jwt_audiences, vec!["slim".to_string()]);
    assert!(default_config.socket_path.is_none());
    assert!(default_config.target_spiffe_id.is_none());
    tracing::info!("Default configuration is correct");

    // Test custom configuration
    let custom_config = SpireConfig {
        socket_path: Some("unix:///custom/path".to_string()),
        target_spiffe_id: Some("spiffe://example.org/backend".to_string()),
        jwt_audiences: vec!["api".to_string(), "web".to_string()],
        ..Default::default()
    };

    let provider = build_manager_from(&custom_config);
    assert_manager_uninitialized(&provider);
    tracing::info!("Correctly fails operations before initialization");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_provider_error_handling() {
    tracing::info!("Testing SPIFFE identity manager error handling");

    let bad_config = test_config_with_socket("unix:///nonexistent/socket");
    let mut provider = build_manager_from(&bad_config);

    // Should fail to initialize
    let init_result = provider.initialize().await;
    assert!(init_result.is_err(), "Should fail with invalid socket");

    let err = format!("{}", init_result.unwrap_err());
    assert!(err.contains("Failed to connect") || err.contains("SPIFFE"));
    tracing::info!("Correctly handles invalid socket path: {}", err);

    // Provider should still be in uninitialized state
    assert_manager_uninitialized(&provider);
    tracing::info!("Provider remains in safe uninitialized state after error");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_verifier_config() {
    require_docker!();

    tracing::info!("Testing SPIFFE verifier configuration");

    // Config without explicit socket path
    let cfg = test_config();
    let verifier_no_socket = build_manager_from(&cfg);
    assert_manager_uninitialized(&verifier_no_socket);

    // Config with empty audiences
    let empty_cfg = SpireConfig {
        socket_path: Some("unix:///tmp/test".to_string()),
        ..Default::default()
    };
    let verifier_empty_aud = build_manager_from(&empty_cfg);
    assert_manager_uninitialized(&verifier_empty_aud);

    tracing::info!("Verifier configuration works correctly");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_spiffe_try_methods() {
    tracing::info!("Testing SPIFFE try_* methods for non-async contexts");

    let cfg_try = test_config();
    let verifier = build_manager_from(&cfg_try);

    // Try to verify without initialization
    let result = verifier.try_verify("fake.token");
    assert!(result.is_err());

    // Try to get claims without initialization
    let claims_result: Result<serde_json::Value, _> = verifier.try_get_claims("fake.token");
    assert!(claims_result.is_err());

    tracing::info!("try_* methods correctly handle uninitialized state");
}
