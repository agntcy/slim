// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SPIRE test environment for integration tests
//!
//! This module provides a reusable test environment that manages SPIRE server and agent
//! containers using bollard (Docker API). It handles the full lifecycle including:
//! - Creating isolated Docker networks
//! - Starting SPIRE server with proper configuration
//! - Generating join tokens and starting SPIRE agent
//! - Registering workload entries
//! - Automatic cleanup of all resources

use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use bollard::network::CreateNetworkOptions;
use futures::StreamExt;
use slim_config::auth::spire::SpireConfig;
use std::collections::HashMap;
use std::time::Duration;
use tokio::fs;

const SPIRE_SERVER_IMAGE: &str = "ghcr.io/spiffe/spire-server";
const SPIRE_AGENT_IMAGE: &str = "ghcr.io/spiffe/spire-agent";
const SPIRE_VERSION: &str = "1.13.2";
const TRUST_DOMAIN: &str = "example.org";

/// Test environment for SPIRE server and agent
///
/// This struct manages the lifecycle of SPIRE containers for integration testing.
/// Each instance creates isolated resources with unique names to support parallel testing.
///
/// # Example
///
/// ```ignore
/// let mut env = SpireTestEnvironment::new().await?;
/// env.start().await?;
///
/// let config = env.get_spiffe_provider_config();
/// // Use config for testing...
///
/// env.cleanup().await;
/// ```
pub struct SpireTestEnvironment {
    docker: Docker,
    network_name: String,
    server_name: String,
    agent_name: String,
    server_container_id: Option<String>,
    agent_container_id: Option<String>,
    temp_dir: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    server_port: Option<u16>,
    dns_name: String,
}

impl SpireTestEnvironment {
    /// Create a new test environment with unique isolated resources
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let docker = Docker::connect_with_local_defaults()?;

        // Create temporary directory for socket and configs with unique ID
        let temp_dir = std::env::temp_dir().join(format!("spire-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).await?;

        // Create socket directory that will be mounted into the container
        let socket_dir = temp_dir.join("socket");
        fs::create_dir_all(&socket_dir).await?;
        let socket_path = socket_dir.join("api.sock");

        // Create unique network name to avoid conflicts
        let network_name = format!("spire-test-{}", uuid::Uuid::new_v4());

        // Create unique server name to avoid conflicts
        let server_name = format!("spire-server-{}", uuid::Uuid::new_v4());

        // Create unique agent name to avoid conflicts
        let agent_name = format!("spire-agent-{}", uuid::Uuid::new_v4());

        // DNS name for workload registration
        let dns_name = format!("testservice.{}", TRUST_DOMAIN);

        Ok(Self {
            docker,
            network_name,
            server_name,
            agent_name,
            server_container_id: None,
            agent_container_id: None,
            temp_dir,
            socket_path,
            server_port: None,
            dns_name,
        })
    }

    /// Start the SPIRE server and agent containers
    ///
    /// This orchestrates the full startup sequence:
    /// 1. Create Docker network
    /// 2. Start SPIRE server and wait for it to be ready
    /// 3. Generate join token
    /// 4. Start SPIRE agent
    /// 5. Register workload entry
    /// 6. Wait for agent to fully connect
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.create_network().await?;
        self.start_server().await?;
        self.start_agent().await?;
        self.register_workload().await?;

        Ok(())
    }

    /// Wait for a specific log message to appear in container logs
    ///
    /// This polls the container logs until the specified message is found or a timeout occurs.
    async fn wait_for_log_message(
        &self,
        container_id: &str,
        message: &str,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Waiting for log message: '{}'", message);

        let logs_options = LogsOptions::<String> {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        };

        let mut log_stream = self.docker.logs(container_id, Some(logs_options));
        let start_time = std::time::Instant::now();

        while let Some(log_result) = log_stream.next().await {
            if start_time.elapsed() > timeout {
                return Err(format!("Timeout waiting for log message: '{}'", message).into());
            }

            if let Ok(log) = log_result {
                let log_str = log.to_string();
                tracing::debug!("Container log: {}", log_str);
                if log_str.contains(message) {
                    tracing::info!("Found log message: '{}'", message);
                    return Ok(());
                }
            }
        }

        Err(format!("Log stream ended before finding message: '{}'", message).into())
    }

    /// Create Docker network for container communication
    async fn create_network(&self) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Creating Docker network: {}", self.network_name);

        let create_network_options = CreateNetworkOptions {
            name: self.network_name.clone(),
            check_duplicate: true,
            ..Default::default()
        };

        self.docker.create_network(create_network_options).await?;
        tracing::info!("Docker network created: {}", self.network_name);

        Ok(())
    }

    /// Pull Docker image if not present
    async fn pull_image(&self, image: &str, tag: &str) -> Result<(), Box<dyn std::error::Error>> {
        let image_name = format!("{}:{}", image, tag);
        tracing::info!("Pulling Docker image: {}", image_name);

        let options = Some(CreateImageOptions {
            from_image: image,
            tag,
            ..Default::default()
        });

        let mut stream = self.docker.create_image(options, None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::debug!("Pull status: {}", status);
                    }
                    if let Some(error) = info.error {
                        return Err(format!("Error pulling image: {}", error).into());
                    }
                }
                Err(e) => return Err(format!("Failed to pull image: {}", e).into()),
            }
        }

        tracing::info!("Successfully pulled image: {}", image_name);
        Ok(())
    }

    /// Start SPIRE server container
    async fn start_server(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Starting SPIRE server container...");

        // Pull the image first
        self.pull_image(SPIRE_SERVER_IMAGE, SPIRE_VERSION).await?;

        // Create server config
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

        let server_config_path = self.temp_dir.join("server.conf");
        fs::write(&server_config_path, server_config).await?;

        // Create container with port mapping
        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            "8081/tcp".to_string(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some("0".to_string()), // Auto-assign port
            }]),
        );

        let server_host_config = HostConfig {
            network_mode: Some(self.network_name.clone()),
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
            name: self.server_name.clone(),
            ..Default::default()
        };

        let server_container = self
            .docker
            .create_container(Some(server_create_options), server_config)
            .await?;

        self.docker
            .start_container::<String>(&server_container.id, None)
            .await?;

        self.server_container_id = Some(server_container.id.clone());

        tracing::info!("SPIRE server container started, waiting for ready signal...");

        // Wait for server to be ready by watching logs
        self.wait_for_log_message(
            &server_container.id,
            "Starting Server APIs",
            Duration::from_secs(30),
        )
        .await?;

        tracing::info!("SPIRE server is ready");

        // Get the mapped port
        let server_inspect = self
            .docker
            .inspect_container(&server_container.id, None)
            .await?;

        self.server_port = server_inspect
            .network_settings
            .as_ref()
            .and_then(|ns| ns.ports.as_ref())
            .and_then(|ports| ports.get("8081/tcp"))
            .and_then(|bindings| bindings.as_ref())
            .and_then(|bindings| bindings.first())
            .and_then(|binding| binding.host_port.as_ref())
            .and_then(|port| port.parse::<u16>().ok());

        tracing::info!("SPIRE server exposed on host port: {:?}", self.server_port);

        Ok(())
    }

    /// Generate join token for agent
    async fn generate_join_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let server_id = self
            .server_container_id
            .as_ref()
            .ok_or("Server container not started")?;

        tracing::info!("Generating join token for agent...");

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

        let exec = self.docker.create_exec(server_id, exec_config).await?;

        let mut token_output = String::new();
        if let StartExecResults::Attached { mut output, .. } =
            self.docker.start_exec(&exec.id, None).await?
        {
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
        Ok(join_token)
    }

    /// Start SPIRE agent container
    async fn start_agent(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Starting SPIRE agent container...");

        // Pull the image first
        self.pull_image(SPIRE_AGENT_IMAGE, SPIRE_VERSION).await?;

        let join_token = self.generate_join_token().await?;

        // Create agent config
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
            server_name = self.server_name,
            trust_domain = TRUST_DOMAIN,
            join_token = join_token
        );

        tracing::info!("SPIRE agent config:\n{}", agent_config);

        let agent_config_path = self.temp_dir.join("agent.conf");
        fs::write(&agent_config_path, agent_config).await?;

        // Prepare bind mounts
        let socket_dir = self.socket_path.parent().unwrap();
        let binds = vec![
            format!(
                "{}:/opt/spire/conf/agent/agent.conf",
                agent_config_path.to_string_lossy()
            ),
            format!("{}:/tmp/spire-agent/public", socket_dir.to_string_lossy()),
        ];

        let host_config = HostConfig {
            pid_mode: Some("host".to_string()),
            network_mode: Some(self.network_name.clone()),
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
            name: self.agent_name.clone(),
            ..Default::default()
        };

        let agent_container = self
            .docker
            .create_container(Some(create_options), config)
            .await?;

        self.docker
            .start_container::<String>(&agent_container.id, None)
            .await?;

        self.agent_container_id = Some(agent_container.id.clone());

        tracing::info!("SPIRE agent container started, waiting for ready signal...");

        // Wait for agent to be ready by watching logs
        self.wait_for_log_message(
            &agent_container.id,
            "Starting Workload and SDS APIs",
            Duration::from_secs(30),
        )
        .await?;

        tracing::info!("SPIRE agent is ready");
        Ok(())
    }

    /// Register workload entry with the current process UID
    async fn register_workload(&self) -> Result<(), Box<dyn std::error::Error>> {
        let server_id = self
            .server_container_id
            .as_ref()
            .ok_or("Server container not started")?;

        tracing::info!("Registering workload with SPIRE server...");

        // Get the current process UID
        #[cfg(unix)]
        let current_uid = unsafe { libc::getuid() };
        let uid_selector = format!("unix:uid:{}", current_uid);

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
                &uid_selector,
                "-dns",
                &self.dns_name,
            ]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        let register_exec = self
            .docker
            .create_exec(server_id, register_exec_config)
            .await?;

        let mut register_output = String::new();
        if let StartExecResults::Attached { mut output, .. } =
            self.docker.start_exec(&register_exec.id, None).await?
        {
            while let Some(Ok(msg)) = output.next().await {
                register_output.push_str(&msg.to_string());
            }
        }

        tracing::info!("Workload registration output: {}", register_output);

        let inspect_exec = self.docker.inspect_exec(&register_exec.id).await?;

        if inspect_exec.exit_code != Some(0) {
            return Err("Failed to register workload".into());
        }

        Ok(())
    }

    /// Get socket path for SPIFFE provider with unix:// prefix
    pub fn socket_path(&self) -> String {
        format!("unix://{}", self.socket_path.to_string_lossy())
    }

    /// Get the DNS name for the target service
    pub fn dns_name(&self) -> &str {
        &self.dns_name
    }

    /// Get a ready-to-use unified SPIFFE config (from slim_config crate)
    pub fn get_spiffe_config(&self) -> SpireConfig {
        SpireConfig {
            socket_path: Some(self.socket_path()),
            jwt_audiences: vec!["test-audience".to_string()],
            trust_domains: vec![TRUST_DOMAIN.to_string()],
            ..Default::default()
        }
    }

    /// Clean up all resources
    ///
    /// Stops and removes all containers, removes the network, and cleans up temporary directories.
    /// This should be called explicitly at the end of each test.
    pub async fn cleanup(&self) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Cleaning up SPIRE test environment");

        let remove_options = Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        });

        // Remove agent container
        if let Some(agent_id) = &self.agent_container_id {
            let _ = self.docker.stop_container(agent_id, None).await;
            let _ = self.docker.remove_container(agent_id, remove_options).await;
        }

        // Remove server container
        if let Some(server_id) = &self.server_container_id {
            let _ = self.docker.stop_container(server_id, None).await;
            let _ = self
                .docker
                .remove_container(server_id, remove_options)
                .await;
        }

        // Remove network
        let _ = self.docker.remove_network(&self.network_name).await;

        // Remove temp directory
        let _ = fs::remove_dir_all(&self.temp_dir).await;

        tracing::info!("Cleanup complete");
        Ok(())
    }
}
