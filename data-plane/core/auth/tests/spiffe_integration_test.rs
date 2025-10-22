// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SPIFFE using real SPIRE server and agent binaries
//!
//! These tests download SPIRE binaries and run them as local processes
//! to test the full authentication flow with real workload API interactions.
//!
//! Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture

#![cfg(not(target_family = "windows"))]

use slim_auth::spiffe::{
    SpiffeJwtVerifier, SpiffeProvider, SpiffeProviderConfig, SpiffeVerifierConfig,
};
use slim_auth::traits::{TokenProvider, Verifier};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::fs;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

const SPIRE_VERSION: &str = "1.13.2";
const TRUST_DOMAIN: &str = "example.org";

/// Get the appropriate SPIRE download URL for the current platform
fn get_spire_download_url() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let platform = match (os, arch) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-amd64",
        ("linux", "x86_64") => "linux-amd64",
        ("linux", "aarch64") => "linux-arm64",
        _ => panic!("Unsupported platform: {} {}", os, arch),
    };

    format!(
        "https://github.com/spiffe/spire/releases/download/v{}/spire-{}-{}.tar.gz",
        SPIRE_VERSION, SPIRE_VERSION, platform
    )
}

/// Download and extract SPIRE binaries
async fn download_spire_binaries(bin_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let url = get_spire_download_url();
    tracing::info!("Downloading SPIRE from: {}", url);

    // Download the tarball
    let response = reqwest::get(&url).await?;
    let bytes = response.bytes().await?;

    // Save to temporary file
    let tarball_path = bin_dir.join("spire.tar.gz");
    fs::write(&tarball_path, &bytes).await?;

    tracing::info!("Extracting SPIRE binaries...");

    // Extract using tar command
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(bin_dir)
        .output()
        .await?;

    if !output.status.success() {
        return Err(format!(
            "Failed to extract SPIRE: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // Remove tarball
    fs::remove_file(&tarball_path).await?;

    tracing::info!("SPIRE binaries extracted successfully");
    Ok(())
}

/// Get the path to SPIRE binaries (download if needed)
async fn get_spire_bin_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir();
    let spire_dir = temp_dir.join(format!("spire-{}", SPIRE_VERSION));
    let extracted_dir = spire_dir.join(format!("spire-{}", SPIRE_VERSION));
    let bin_dir = extracted_dir.join("bin");

    let server_bin = bin_dir.join("spire-server");
    let agent_bin = bin_dir.join("spire-agent");

    // Check if binaries already exist
    if server_bin.exists() && agent_bin.exists() {
        tracing::info!("SPIRE binaries already downloaded");
        return Ok(bin_dir);
    }

    // Create directory and download
    fs::create_dir_all(&spire_dir).await?;
    download_spire_binaries(&spire_dir).await?;

    Ok(bin_dir)
}

struct SpireTestEnvironment {
    server_process: Child,
    agent_process: Child,
    data_dir: PathBuf,
    socket_path: PathBuf,
}

impl SpireTestEnvironment {
    async fn setup() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = std::env::temp_dir();
        let test_id = uuid::Uuid::new_v4();
        let test_dir = temp_dir.join(format!("spire-test-{}", test_id));
        
        fs::create_dir_all(&test_dir).await?;

        let data_dir = test_dir.join("data");
        let server_data = data_dir.join("server");
        let agent_data = data_dir.join("agent");
        let socket_dir = test_dir.join("socket");
        
        fs::create_dir_all(&server_data).await?;
        fs::create_dir_all(&agent_data).await?;
        fs::create_dir_all(&socket_dir).await?;

        let socket_path = socket_dir.join("agent.sock");
        let server_socket = socket_dir.join("registration.sock");

        // Get SPIRE binaries
        let bin_dir = get_spire_bin_dir().await?;
        let server_bin = bin_dir.join("spire-server");
        let agent_bin = bin_dir.join("spire-agent");

        // Create server config
        let server_config = format!(
            r#"
server {{
    bind_address = "127.0.0.1"
    bind_port = "8081"
    socket_path = "{}"
    trust_domain = "{}"
    data_dir = "{}"
    log_level = "DEBUG"
    ca_ttl = "1h"
    default_x509_svid_ttl = "5m"
    default_jwt_svid_ttl = "5m"
}}

plugins {{
    DataStore "sql" {{
        plugin_data {{
            database_type = "sqlite3"
            connection_string = "{}/datastore.sqlite3"
        }}
    }}

    KeyManager "memory" {{
        plugin_data {{}}
    }}

    NodeAttestor "join_token" {{
        plugin_data {{}}
    }}
}}
"#,
            server_socket.display(),
            TRUST_DOMAIN,
            server_data.display(),
            server_data.display()
        );

        let server_config_path = test_dir.join("server.conf");
        fs::write(&server_config_path, server_config).await?;

        tracing::info!("Starting SPIRE server...");
        
        // Start server
        let mut server_process = Command::new(&server_bin)
            .arg("run")
            .arg("-config")
            .arg(&server_config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Wait for server to be ready
        sleep(Duration::from_secs(3)).await;

        // Check if server is still running
        if let Ok(Some(status)) = server_process.try_wait() {
            return Err(format!("SPIRE server exited early with status: {}", status).into());
        }

        tracing::info!("SPIRE server started");

        // Generate join token
        tracing::info!("Generating join token...");
        let token_output = Command::new(&server_bin)
            .arg("token")
            .arg("generate")
            .arg("-socketPath")
            .arg(&server_socket)
            .arg("-spiffeID")
            .arg(format!("spiffe://{}/testagent", TRUST_DOMAIN))
            .output()
            .await?;

        if !token_output.status.success() {
            return Err(format!(
                "Failed to generate join token: {}",
                String::from_utf8_lossy(&token_output.stderr)
            )
            .into());
        }

        let token_str = String::from_utf8_lossy(&token_output.stdout);
        let join_token = token_str
            .lines()
            .find(|line| line.starts_with("Token:"))
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or("Failed to parse join token")?
            .to_string();

        tracing::info!("Join token generated: {}", join_token);

        // Create agent config
        let agent_config = format!(
            r#"
agent {{
    data_dir = "{}"
    log_level = "DEBUG"
    server_address = "127.0.0.1"
    server_port = "8081"
    socket_path = "{}"
    trust_domain = "{}"
    insecure_bootstrap = true
    join_token = "{}"
}}

plugins {{
    KeyManager "memory" {{
        plugin_data {{}}
    }}

    NodeAttestor "join_token" {{
        plugin_data {{}}
    }}

    WorkloadAttestor "unix" {{
        plugin_data {{}}
    }}
}}
"#,
            agent_data.display(),
            socket_path.display(),
            TRUST_DOMAIN,
            join_token
        );

        let agent_config_path = test_dir.join("agent.conf");
        fs::write(&agent_config_path, agent_config).await?;

        tracing::info!("Starting SPIRE agent...");

        // Start agent
        let agent_process = Command::new(&agent_bin)
            .arg("run")
            .arg("-config")
            .arg(&agent_config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Wait for agent to be ready
        sleep(Duration::from_secs(3)).await;

        tracing::info!("SPIRE agent started");

        // Register workload
        tracing::info!("Registering workload...");
        
        // Get current UID for the selector
        let uid = unsafe { libc::getuid() };
        
        let register_output = Command::new(&server_bin)
            .arg("entry")
            .arg("create")
            .arg("-socketPath")
            .arg(&server_socket)
            .arg("-parentID")
            .arg(format!("spiffe://{}/testagent", TRUST_DOMAIN))
            .arg("-spiffeID")
            .arg(format!("spiffe://{}/myservice", TRUST_DOMAIN))
            .arg("-selector")
            .arg(format!("unix:uid:{}", uid))
            .output()
            .await?;

        if !register_output.status.success() {
            tracing::warn!(
                "Workload registration warning: {}",
                String::from_utf8_lossy(&register_output.stderr)
            );
        } else {
            tracing::info!("Workload registered successfully");
        }

        // Give everything a moment to settle
        sleep(Duration::from_secs(2)).await;

        Ok(Self {
            server_process,
            agent_process,
            data_dir,
            socket_path,
        })
    }

    async fn cleanup(mut self) {
        tracing::info!("Cleaning up SPIRE environment...");
        
        // Kill processes
        let _ = self.agent_process.kill().await;
        let _ = self.server_process.kill().await;
        
        // Wait for them to exit
        let _ = self.agent_process.wait().await;
        let _ = self.server_process.wait().await;

        // Clean up data directory
        if let Some(parent) = self.data_dir.parent() {
            let _ = fs::remove_dir_all(parent).await;
        }
        
        tracing::info!("Cleanup complete");
    }
}

#[tokio::test]
#[ignore] // Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture
#[tracing_test::traced_test]
async fn test_spiffe_provider_initialization() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    // Configure SPIFFE provider
    let config = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let mut provider = SpiffeProvider::new(config);

    // Initialize provider
    let init_result = provider.initialize().await;
    assert!(
        init_result.is_ok(),
        "Failed to initialize provider: {:?}",
        init_result.err()
    );

    tracing::info!("Provider initialized successfully");

    // Get X.509 SVID
    let x509_result = provider.get_x509_svid();
    assert!(x509_result.is_ok(), "Failed to get X.509 SVID");
    
    let svid = x509_result.unwrap();
    tracing::info!("Got X.509 SVID with SPIFFE ID: {}", svid.spiffe_id());

    // Get X.509 certificate in PEM format
    let cert_pem = provider.get_x509_cert_pem();
    assert!(cert_pem.is_ok(), "Failed to get certificate PEM");
    assert!(cert_pem.unwrap().contains("BEGIN CERTIFICATE"));

    // Get X.509 private key in PEM format
    let key_pem = provider.get_x509_key_pem();
    assert!(key_pem.is_ok(), "Failed to get private key PEM");
    assert!(key_pem.unwrap().contains("BEGIN PRIVATE KEY"));

    // Get JWT token
    let token_result = provider.get_token();
    assert!(token_result.is_ok(), "Failed to get JWT token");
    
    let token = token_result.unwrap();
    tracing::info!("Got JWT token: {}...", &token[..50.min(token.len())]);

    // Get SPIFFE ID
    let id_result = provider.get_id();
    assert!(id_result.is_ok(), "Failed to get SPIFFE ID");
    
    let spiffe_id = id_result.unwrap();
    tracing::info!("SPIFFE ID: {}", spiffe_id);
    assert!(spiffe_id.starts_with("spiffe://"));

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_jwt_verifier_creation() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    // Create provider to generate a token
    let provider_config = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let mut provider = SpiffeProvider::new(provider_config);
    provider.initialize().await.expect("Failed to initialize provider");

    // Create verifier
    let verifier_config = SpiffeVerifierConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let verifier = SpiffeJwtVerifier::new(verifier_config);
    verifier.initialize().await.expect("Failed to initialize verifier");

    // Get a token from the provider
    let token = provider.get_token().expect("Failed to get token");

    // Verify the token
    sleep(Duration::from_secs(1)).await; // Give verifier time to get bundles
    
    let verify_result = verifier.verify(token.clone()).await;
    assert!(
        verify_result.is_ok(),
        "Failed to verify token: {:?}",
        verify_result.err()
    );

    tracing::info!("Token verified successfully");

    // Get claims from token
    let claims_result: Result<serde_json::Value, _> = verifier.get_claims(token).await;
    assert!(claims_result.is_ok(), "Failed to get claims");
    
    let claims = claims_result.unwrap();
    tracing::info!("Claims: {:?}", claims);

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_provider_configurations() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    // Test with custom audiences
    let config = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["custom-aud1".to_string(), "custom-aud2".to_string()],
    };

    let mut provider = SpiffeProvider::new(config);
    assert!(provider.initialize().await.is_ok());

    let token_result = provider.get_token();
    assert!(token_result.is_ok());

    tracing::info!("Provider with custom audiences works");

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_provider_error_handling() {
    // Test with invalid socket path
    let config = SpiffeProviderConfig {
        socket_path: Some("/invalid/socket/path.sock".to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let mut provider = SpiffeProvider::new(config);
    let init_result = provider.initialize().await;
    
    assert!(init_result.is_err(), "Expected error with invalid socket path");
    tracing::info!("Correctly handled invalid socket path");
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_multiple_providers_isolation() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    // Create two providers with different audiences
    let config1 = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["audience1".to_string()],
    };

    let config2 = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["audience2".to_string()],
    };

    let mut provider1 = SpiffeProvider::new(config1);
    let mut provider2 = SpiffeProvider::new(config2);

    assert!(provider1.initialize().await.is_ok());
    assert!(provider2.initialize().await.is_ok());

    let token1 = provider1.get_token().expect("Provider 1 failed");
    let token2 = provider2.get_token().expect("Provider 2 failed");

    // Tokens should be different (different audiences)
    assert_ne!(token1, token2);

    tracing::info!("Multiple providers work independently");

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_config_validation() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    // Test with empty audiences
    let config = SpiffeProviderConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec![],
    };

    let mut provider = SpiffeProvider::new(config);
    
    // Should still initialize, but might have issues getting tokens
    assert!(provider.initialize().await.is_ok());

    tracing::info!("Config validation works");

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_verifier_config() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    let verifier_config = SpiffeVerifierConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let verifier = SpiffeJwtVerifier::new(verifier_config);
    assert!(verifier.initialize().await.is_ok());

    tracing::info!("Verifier configuration works");

    env.cleanup().await;
}

#[tokio::test]
#[ignore]
#[tracing_test::traced_test]
async fn test_spiffe_try_methods() {
    let env = SpireTestEnvironment::setup()
        .await
        .expect("Failed to setup SPIRE environment");

    let verifier_config = SpiffeVerifierConfig {
        socket_path: Some(env.socket_path.to_string_lossy().to_string()),
        jwt_audiences: vec!["test-audience".to_string()],
    };

    let verifier = SpiffeJwtVerifier::new(verifier_config);
    verifier.initialize().await.expect("Failed to initialize");

    // Wait for bundles to be available
    sleep(Duration::from_secs(2)).await;

    // Try with invalid token - should fail quickly
    let result = verifier.try_verify("invalid.token");
    assert!(result.is_err());

    tracing::info!("Try methods work correctly");

    env.cleanup().await;
}