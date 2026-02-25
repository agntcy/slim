// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::{ArgGroup, Args, Subcommand};

use crate::defaults::DEFAULT_ENDPOINT;

#[derive(Args)]
pub struct SlimArgs {
    #[command(subcommand)]
    pub command: SlimCommand,
}

#[derive(Subcommand)]
pub enum SlimCommand {
    /// Start a local SLIM instance
    Start(SlimStartArgs),
}

#[derive(Args)]
#[command(group(ArgGroup::new("source")))]
pub struct SlimStartArgs {
    /// Path to YAML configuration file
    #[arg(short = 'c', long, group = "source")]
    pub config: Option<String>,
    /// Server endpoint override
    #[arg(long, env = "SLIMCTL_SLIM_ENDPOINT", group = "source")]
    pub endpoint: Option<String>,
}

pub async fn run(args: &SlimArgs) -> Result<()> {
    match &args.command {
        SlimCommand::Start(start_args) => run_start(start_args).await,
    }
}

async fn run_start(args: &SlimStartArgs) -> Result<()> {
    // Resolve effective endpoint: explicit flag > default (when no config file given)
    let effective_endpoint = args
        .endpoint
        .as_deref()
        .or_else(|| args.config.is_none().then_some(DEFAULT_ENDPOINT));

    let config_file = args.config.as_deref().unwrap_or("");

    // If no config file, create a minimal default config using the endpoint
    let temp_config;
    let config_path = if config_file.is_empty() {
        temp_config = create_temp_config(effective_endpoint)?;
        temp_config
            .path()
            .to_str()
            .context("temp config path is not UTF-8")?
            .to_string()
    } else {
        config_file.to_string()
    };

    // Load and start the SLIM service using the slim crate's machinery
    start_slim_instance(&config_path).await
}

fn create_temp_config(endpoint: Option<&str>) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let endpoint_str = endpoint.unwrap_or(DEFAULT_ENDPOINT);
    let config_yaml = format!(
        r#"runtime:
  n_cores: 0
  thread_name: "slim-worker"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "{}"
          tls:
            insecure: true
      clients: []
"#,
        endpoint_str
    );

    let mut tmp = tempfile::NamedTempFile::new().context("failed to create temp config file")?;
    tmp.write_all(config_yaml.as_bytes())
        .context("failed to write temp config")?;
    Ok(tmp)
}

fn run_slim_in_thread(config_file: &str) -> Result<()> {
    let mut config =
        slim::config::ConfigLoader::new(config_file).context("failed to load configuration")?;

    slim_config::tls::provider::initialize_crypto_provider();

    let runtime = slim::runtime::build(config.runtime().context("invalid runtime configuration")?);
    runtime.runtime.block_on(slim::runner::run_services(config))
}

async fn start_slim_instance(config_file: &str) -> Result<()> {
    let config_path = config_file.to_string();

    // Run the slim instance in a dedicated OS thread so that we can build a
    // fresh Tokio runtime sized and named exactly as the config specifies,
    // without interfering with slimctl's own runtime.
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("slim-runtime".to_string())
        .spawn(move || {
            let _ = tx.send(run_slim_in_thread(&config_path));
        })
        .context("failed to spawn slim runtime thread")?;

    rx.await
        .context("slim instance thread terminated unexpectedly")?
}
