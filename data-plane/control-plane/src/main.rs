// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use slim_config::provider::ConfigResolver;
use slim_config::tls::provider::initialize_crypto_provider;

use slim_control_plane::config::Config;
use slim_control_plane::server::ControlPlane;

#[derive(Debug, Parser)]
#[command(name = "slim-control-plane", about = "SLIM control plane server")]
struct Args {
    /// Path to the YAML configuration file.
    /// If not given, the process looks for "config.yaml" in the working directory.
    /// If that file is absent too, built-in defaults are used.
    #[arg(short, long, value_name = "FILE")]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    initialize_crypto_provider();

    let mut cfg = load_config(args.config)?;

    let _tracing_guard = std::mem::take(&mut cfg.tracing)
        .setup_tracing_subscriber()
        .context("failed to initialise tracing")?;

    tracing::info!("northbound endpoint: {}", cfg.northbound.endpoint);
    tracing::info!("southbound endpoint: {}", cfg.southbound.endpoint);

    let cp = ControlPlane::start(cfg).await?;

    tracing::info!("control plane started");
    slim_signal::shutdown().await;
    tracing::info!("shutdown signal received, draining connections");

    if tokio::time::timeout(std::time::Duration::from_secs(30), cp.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("shutdown timed out after 30s, forcing exit");
    }

    tracing::info!("control plane stopped");
    Ok(())
}

/// Load configuration from a file, falling back to "config.yaml" (if it
/// exists), then to built-in defaults — matching Go's OverrideFromFile logic.
///
/// `${env:VAR}` and `${file:PATH}` references in YAML values are resolved
/// before deserialisation, matching the pattern used by SLIM.
fn load_config(path: Option<String>) -> Result<Config> {
    let explicit = path.is_some();
    let path = path.unwrap_or_else(|| "config.yaml".to_string());

    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let mut root: serde_yaml::Value = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse config file: {path}"))?;
            ConfigResolver::new()
                .resolve(&mut root)
                .with_context(|| format!("failed to resolve config variables in: {path}"))?;
            serde_yaml::from_value(root)
                .with_context(|| format!("failed to deserialise config file: {path}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
            tracing::debug!("no config file found at {path}, using defaults");
            Ok(Config::default())
        }
        Err(e) => Err(e).with_context(|| format!("failed to read config file: {path}")),
    }
}
