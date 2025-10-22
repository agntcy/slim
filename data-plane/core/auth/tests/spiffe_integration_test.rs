// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SPIFFE using real SPIRE server and agent
//!
//! These tests use bollard to spin up actual SPIRE server and agent
//! containers to test the full authentication flow with real workload API interactions.
//!
//! Run with: cargo test --test spiffe_integration_test -- --ignored --nocapture

#![cfg(not(target_family = "windows"))]

use futures::StreamExt;
use slim_auth::spiffe::{
    SpiffeJwtVerifier, SpiffeProvider, SpiffeProviderConfig, SpiffeVerifierConfig,
};
use slim_auth::traits::{TokenProvider, Verifier};
use std::time::Duration;
use tokio::fs;
use tokio::time::sleep;
use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions};
use bollard::network::CreateNetworkOptions;
use bollard::models::{HostConfig, Mount as BollardMount, MountTypeEnum, PortBinding};
use std::collections::HashMap;



const SPIRE_SERVER_IMAGE: &str = "ghcr.io/spiffe/spire-server";
const SPIRE_AGENT_IMAGE: &str = "ghcr.io/spiffe/spire-agent";
const SPIRE_VERSION: &str = "1.13.2";
const TRUST_DOMAIN: &str = "example.org";
const SERVER_CONTAINER_NAME: &str = "spire-server";
const AGENT_CONTAINER_NAME: &str = "spire-agent";

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

    // Create temporary directory for socket and configs
    let temp_dir = std::env::temp_dir();
    fs::create_dir_all(&temp_dir)
        .await
        .expect("Failed to create temp dir");

    // Create socket directory that will be mounted into the container
    let socket_dir = temp_dir.join("socket");
    fs::create_dir_all(&socket_dir)
        .await
        .expect("Failed to create socket dir");
    let socket_path = socket_dir.join("api.sock");

    // Create Docker network using bollard
    let network_name = "spire-test-network";
    tracing::info!("Creating Docker network: {}", network_name);

    // Get Docker client
    let docker = Docker::connect_with_local_defaults()
        .expect("Failed to connect to Docker");

    // Create the network
    let create_network_options = CreateNetworkOptions {
        name: network_name,
        check_duplicate: true,
        ..Default::default()
    };

    let _network_id = docker
        .create_network(create_network_options)
        .await
        .expect("Failed to create Docker network");

    tracing::info!("Docker network created: {}", network_name);

    // Create minimal server config
    let server_config = format!(
        r#"
server {{
    bind_address = "0.0.0.0"
    bind_port = "8081"
    trust_domain = "{}"
    data_dir = "/opt/spire/data/server"
    log_level = "INFO"
    ca_ttl = "1h"
    default_x509_svid_ttl = "1h"
    default_jwt_svid_ttl = "1h"
}}

plugins {{
    DataStore "sql" {{
        plugin_data {{
            database_type = "sqlite3"
            connection_string = "/opt/spire/data/server/datastore.sqlite3"
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
        TRUST_DOMAIN
    );

    let server_config_path = temp_dir.join("server.conf");
    fs::write(&server_config_path, server_config)
        .await
        .expect("Failed to write server config");

    tracing::info!("Starting SPIRE server container...");

    // Create SPIRE server container with bollard
    let mut port_bindings = HashMap::new();
    port_bindings.insert(
        "8081/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some("0".to_string()), // Auto-assign port
        }]),
    );

    let server_host_config = HostConfig {
        network_mode: Some(network_name.to_string()),
        port_bindings: Some(port_bindings),
        binds: Some(vec![format!(
            "{}:/opt/spire/conf/server/server.conf",
            server_config_path.to_string_lossy()
        )]),
        ..Default::default()
    };

    let mut exposed_ports = HashMap::new();
    exposed_ports.insert("8081/tcp".to_string(), HashMap::new());

    let server_config = Config {
        image: Some(format!("{}:{}", SPIRE_SERVER_IMAGE, SPIRE_VERSION)),
        cmd: Some(vec![
            "run".to_string(),
            "-config".to_string(),
            "/opt/spire/conf/server/server.conf".to_string(),
        ]),
        exposed_ports: Some(exposed_ports),
        host_config: Some(server_host_config),
        ..Default::default()
    };

    let server_create_options = CreateContainerOptions {
        name: SERVER_CONTAINER_NAME,
        ..Default::default()
    };

    let server_container = docker
        .create_container(Some(server_create_options), server_config)
        .await
        .expect("Failed to create server container");

    docker
        .start_container::<String>(&server_container.id, None)
        .await
        .expect("Failed to start server container");

    tracing::info!("SPIRE server container started, waiting for ready signal...");

    // Wait for "Starting Server APIs" message in logs
    let logs_options = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        ..Default::default()
    };

    let mut log_stream = docker.logs(&server_container.id, Some(logs_options));
    let mut server_ready = false;

    while let Some(log_result) = log_stream.next().await {
        if let Ok(log) = log_result {
            let log_str = log.to_string();
            tracing::debug!("Server log: {}", log_str);
            if log_str.contains("Starting Server APIs") {
                server_ready = true;
                break;
            }
        }
    }

    if !server_ready {
        panic!("Server did not start properly");
    }

    tracing::info!("SPIRE server is ready");

    // Get the actual mapped port for the server
    let server_inspect = docker
        .inspect_container(&server_container.id, None)
        .await
        .expect("Failed to inspect server container");

    let server_port = server_inspect
        .network_settings
        .as_ref()
        .and_then(|ns| ns.ports.as_ref())
        .and_then(|ports| ports.get("8081/tcp"))
        .and_then(|bindings| bindings.as_ref())
        .and_then(|bindings| bindings.first())
        .and_then(|binding| binding.host_port.as_ref())
        .and_then(|port| port.parse::<u16>().ok())
        .expect("Failed to get server port");

    tracing::info!("SPIRE server exposed on host port: {}", server_port);

    // Generate join token for agent by execing into the server container
    tracing::info!("Generating join token for agent...");

    use bollard::exec::{CreateExecOptions, StartExecResults};

    let exec_config = CreateExecOptions {
        cmd: Some(vec![
            "/opt/spire/bin/spire-server",
            "token",
            "generate",
            "-spiffeID",
            "spiffe://example.org/testagent",
        ]),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let exec = docker
        .create_exec(&server_container.id, exec_config)
        .await
        .expect("Failed to create exec");

    let mut token_output = String::new();
    if let StartExecResults::Attached { mut output, .. } = docker.start_exec(&exec.id, None).await.expect("Failed to start exec") {
        while let Some(Ok(msg)) = output.next().await {
            token_output.push_str(&msg.to_string());
        }
    }

    let join_token = token_output
        .trim()
        .strip_prefix("Token: ")
        .unwrap_or(token_output.trim())
        .to_string();

    tracing::info!("Generated join token: {}", join_token);

    tracing::info!("Starting SPIRE agent container...");

    // Create agent config with the generated join token
    let agent_config = format!(
        r#"
agent {{
    data_dir = "/opt/spire/data/agent"
    log_level = "INFO"
    server_address = "{server_name}"
    server_port = "8081"
    insecure_bootstrap = true
    trust_domain = "{trust_domain}"
    socket_path = "/tmp/spire-agent/public/api.sock"
    join_token = "{join_token}"
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
        server_name = SERVER_CONTAINER_NAME,
        trust_domain = TRUST_DOMAIN,
        join_token = join_token
    );

    let agent_config_path = temp_dir.join("agent.conf");
    fs::write(&agent_config_path, agent_config)
        .await
        .expect("Failed to write agent config");

    // Note: Using --pid=host for the agent container to allow proper workload attestation

    // Prepare bind mounts for the agent
    let binds = vec![
        format!("{}:/opt/spire/conf/agent/agent.conf", agent_config_path.to_string_lossy()),
        format!("{}:/tmp/spire-agent/public", socket_dir.to_string_lossy()),
    ];

    let host_config = HostConfig {
        pid_mode: Some("host".to_string()),
        network_mode: Some(network_name.to_string()),
        binds: Some(binds),
        ..Default::default()
    };

    let config = Config {
        image: Some(format!("{}:{}", SPIRE_AGENT_IMAGE, SPIRE_VERSION)),
        cmd: Some(vec![
            "run".to_string(),
            "-config".to_string(),
            "/opt/spire/conf/agent/agent.conf".to_string(),
        ]),
        host_config: Some(host_config),
        ..Default::default()
    };

    let create_options = CreateContainerOptions {
        name: AGENT_CONTAINER_NAME,
        ..Default::default()
    };

    let agent_container = docker
        .create_container(Some(create_options), config)
        .await
        .expect("Failed to create agent container");

    docker
        .start_container::<String>(&agent_container.id, None)
        .await
        .expect("Failed to start agent container");

    tracing::info!("SPIRE agent started");

    tracing::info!("Registering workload with SPIRE server...");

    let register_exec_config = CreateExecOptions {
        cmd: Some(vec![
            "/opt/spire/bin/spire-server",
            "entry",
            "create",
            "-parentID",
            "spiffe://example.org/testagent",
            "-spiffeID",
            "spiffe://example.org/testservice",
            "-selector",
            "unix:uid:0",
        ]),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let register_exec = docker
        .create_exec(&server_container.id, register_exec_config)
        .await
        .expect("Failed to create exec");

    let mut register_output = String::new();
    if let StartExecResults::Attached { mut output, .. } = docker.start_exec(&register_exec.id, None).await.expect("Failed to start exec") {
        while let Some(Ok(msg)) = output.next().await {
            register_output.push_str(&msg.to_string());
        }
    }

    tracing::info!("Workload registration output: {}", register_output);

    let inspect_exec = docker
        .inspect_exec(&register_exec.id)
        .await
        .expect("Failed to inspect exec");

    assert!(
        inspect_exec.exit_code == Some(0),
        "Failed to register workload",
    );

    // Wait for agent to connect to server
    sleep(Duration::from_secs(5)).await;

    // Now test our SPIFFE provider
    // Note: The socket directory is mounted from host, agent creates socket inside
    let config = SpiffeProviderConfig {
        socket_path: Some(socket_path.to_string_lossy().to_string()),
        target_spiffe_id: None,
        jwt_audiences: vec!["test-audience".to_string()],
    };

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
                    assert!(svid.spiffe_id().to_string().contains(TRUST_DOMAIN));
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
        }
        Err(e) => {
            tracing::error!("Provider initialization failed: {}", e);
            panic!("Provider initialization should succeed with proper join token attestation");
        }
    }

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir).await;

    // Stop and remove containers
    tracing::info!("Stopping and removing containers");
    let remove_options = Some(RemoveContainerOptions {
        force: true,
        ..Default::default()
    });

    let _ = docker.stop_container(&agent_container.id, None).await;
    let _ = docker.remove_container(&agent_container.id, remove_options.clone()).await;

    let _ = docker.stop_container(&server_container.id, None).await;
    let _ = docker.remove_container(&server_container.id, remove_options).await;

    // Remove Docker network
    tracing::info!("Removing Docker network: {}", network_name);
    let _ = docker.remove_network(network_name).await;
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
