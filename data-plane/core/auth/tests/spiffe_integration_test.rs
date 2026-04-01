// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SPIFFE using real SPIRE server and agent
//!
//! These tests use bollard to spin up actual SPIRE server and agent
//! containers to test the full authentication flow with real workload API interactions.
//!
//! Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture

#![cfg(target_os = "linux")]

use display_error_chain::ErrorChainExt;
use slim_auth::errors::AuthError;
use slim_auth::identity_claims::IdentityClaims;
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

    builder.build().expect("SpireIdentityManager build failed")
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
    tracing::info!(?config, "Creating SpiffeIdentityManager");

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
                tracing::error!(error = %e.chain(), "provider initialization failed");
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
                tracing::error!(error = %e.chain(), "verifier initialization failed");
                should_panic = true;
                break 'test_block;
            }
        }

        // Test X.509 bundle retrieval
        match verifier.get_x509_bundle() {
            Ok(x509_bundle) => {
                tracing::info!(
                    trust_domain = %x509_bundle.trust_domain(), "Successfully retrieved X.509 bundle",

                );
                // Verify the bundle has authorities (CA certificates)
                let authorities = x509_bundle.authorities();
                tracing::info!(n_certificates = %authorities.len(), "CA certificate(s) in bundle");
                assert!(
                    !authorities.is_empty(),
                    "Bundle should contain at least one CA certificate"
                );
            }
            Err(e) => {
                tracing::error!(error = %e.chain(), "failed to get X.509 bundle");
                should_panic = true;
                break 'test_block;
            }
        }

        // Test X.509 SVID retrieval
        match provider.get_x509_svid() {
            Ok(svid) => {
                tracing::info!(spiffe_id = %svid.spiffe_id(), "Got X.509 SVID");
                assert!(svid.spiffe_id().to_string().contains("example.org"));
            }
            Err(e) => {
                tracing::error!(error = %e.chain(), "x.509 SVID fetch failed");
                should_panic = true;
                break 'test_block;
            }
        }

        // Test JWT token retrieval
        let token = match provider.get_token() {
            Ok(token) => {
                tracing::info!("Got JWT token");
                assert!(!token.is_empty());
                let parts: Vec<&str> = token.split('.').collect();
                assert_eq!(parts.len(), 3, "JWT should have 3 parts");
                token
            }
            Err(e) => {
                tracing::error!(error = %e.chain(), "jwt token fetch failed");
                should_panic = true;
                break 'test_block;
            }
        };

        // Verify that the token contains the MLS public key in custom_claims
        match verifier.get_claims::<serde_json::Value>(token).await {
            Ok(claims) => {
                tracing::info!(%claims, "Successfully verified JWT token");
                match IdentityClaims::from_json(&claims) {
                    Ok(identity) => {
                        assert!(
                            !identity.public_key.is_empty(),
                            "pubkey should not be empty"
                        );
                        assert!(!identity.subject.is_empty(), "subject should not be empty");
                        tracing::info!(
                            pubkey = %identity.public_key,
                            subject = %identity.subject,
                            "Found MLS identity claims in JWT"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to extract identity claims from JWT");
                        should_panic = true;
                        break 'test_block;
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e.chain(), "JWT verification failed");
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
    assert!(
        init_result.is_err_and(|e| matches!(e, AuthError::SpiffeGrpcError(_))),
        "Should fail with invalid socket"
    );

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

/// Tests the `Ok(svid)` arm of `CachedJwtSvid::background_refresh`.
///
/// A 45-second JWT TTL means the 30-second initial delay is short enough to wait
/// for in a test.  After the delay the background task fetches a new SVID, updates
/// the cache, and logs `"jwt_source: SVID refreshed"`.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_jwt_svid_background_refresh_success() {
    require_docker!();

    let mut env = SpireTestEnvironment::new()
        .await
        .expect("Failed to create test environment")
        .with_jwt_svid_ttl("45s");
    env.start().await.expect("Failed to start SPIRE containers");

    let config = env.get_spiffe_config();
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let mut provider = build_manager_from(&config);
    let mut should_panic = false;

    'test_block: {
        if let Err(e) = provider.initialize().await {
            tracing::error!(error = %e, "Provider initialization failed");
            should_panic = true;
            break 'test_block;
        }

        let initial_token = match provider.get_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get initial token");
                should_panic = true;
                break 'test_block;
            }
        };

        // Wait for background_refresh initial delay (30 s) + small margin.
        tracing::info!("Waiting 32 s for background SVID refresh...");
        tokio::time::sleep(std::time::Duration::from_secs(32)).await;

        let refreshed_token = match provider.get_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get token after refresh");
                should_panic = true;
                break 'test_block;
            }
        };

        assert_ne!(
            initial_token, refreshed_token,
            "background_refresh should have replaced the cached SVID"
        );
        assert!(
            logs_contain("jwt_source: SVID refreshed"),
            "expected success log from the Ok arm of background_refresh"
        );

        break 'test_block;
    }

    env.cleanup().await.expect("Failed to cleanup");
    if should_panic {
        panic!("test_jwt_svid_background_refresh_success failed");
    }
}

/// Tests the `Err(err)` arm of `CachedJwtSvid::background_refresh`, including
/// the `calculate_backoff_with_token_expiry` call.
///
/// After a successful initialisation the SPIRE agent is stopped.  When the 30-second
/// initial delay elapses the background task attempts a refresh, fails, and logs the
/// backing-off warning.  The previously-cached SVID must remain accessible.
#[tokio::test]
#[tracing_test::traced_test]
async fn test_jwt_svid_background_refresh_error() {
    require_docker!();

    let mut env = SpireTestEnvironment::new()
        .await
        .expect("Failed to create test environment");
    env.start().await.expect("Failed to start SPIRE containers");

    let config = env.get_spiffe_config();
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let mut provider = build_manager_from(&config);
    let mut should_panic = false;

    'test_block: {
        if let Err(e) = provider.initialize().await {
            tracing::error!(error = %e, "Provider initialization failed");
            should_panic = true;
            break 'test_block;
        }

        let initial_token = match provider.get_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get initial token");
                should_panic = true;
                break 'test_block;
            }
        };

        // Bring down the agent so the next refresh attempt fails.
        if let Err(e) = env.stop_agent().await {
            tracing::error!(error = %e, "Failed to stop SPIRE agent");
            should_panic = true;
            break 'test_block;
        }

        // Wait for the background_refresh initial delay (30 s) + small margin.
        tracing::info!("Waiting 32 s for background SVID refresh attempt...");
        tokio::time::sleep(std::time::Duration::from_secs(32)).await;

        assert!(
            logs_contain("jwt_source: SVID refresh failed; backing off"),
            "expected backoff warning from the Err arm of background_refresh"
        );

        // The cached token must be unchanged — the Err arm must not clear the cache.
        let token_after_failure = match provider.get_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get token after refresh failure");
                should_panic = true;
                break 'test_block;
            }
        };
        assert_eq!(
            initial_token, token_after_failure,
            "cached SVID should be unchanged after a failed refresh"
        );

        break 'test_block;
    }

    env.cleanup().await.expect("Failed to cleanup");
    if should_panic {
        panic!("test_jwt_svid_background_refresh_error failed");
    }
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
