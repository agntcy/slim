// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0


use anyhow::{Context, Result};
use clap::Parser;
use slim_config::provider::ConfigResolver;
use slim_config::tls::provider::initialize_crypto_provider;

use slim_control_plane::api::proto::controller::proto::v1::controller_service_server::ControllerServiceServer;
use slim_control_plane::api::proto::controlplane::proto::v1::control_plane_service_server::ControlPlaneServiceServer;
use slim_control_plane::config::Config;
use slim_control_plane::node_control::DefaultNodeCommandHandler;
use slim_control_plane::services::group::GroupService;
use slim_control_plane::services::northbound::NorthboundApiService;
use slim_control_plane::services::routes::RouteService;
use slim_control_plane::services::southbound::SouthboundApiService;

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

    // ── Initialise crypto provider ───────────────────────────────────────────
    initialize_crypto_provider();

    // ── Load config ──────────────────────────────────────────────────────────
    let cfg = load_config(args.config)?;

    // ── Initialise tracing ───────────────────────────────────────────────────
    let _tracing_guard = cfg
        .tracing
        .setup_tracing_subscriber()
        .context("failed to initialise tracing")?;

    tracing::info!("northbound endpoint: {}", cfg.northbound.endpoint);
    tracing::info!("southbound endpoint: {}", cfg.southbound.endpoint);

    // ── Database backend ─────────────────────────────────────────────────────
    let db = slim_control_plane::db::open(&cfg.database).await?;

    // ── Core services ─────────────────────────────────────────────────────────
    let cmd_handler = DefaultNodeCommandHandler::new();

    let route_service = RouteService::new(
        db.clone(),
        cmd_handler.clone(),
        cfg.reconciler,
    );

    let group_service = GroupService::new(db.clone(), cmd_handler.clone());

    let nb_svc = NorthboundApiService::new(
        db.clone(),
        cmd_handler.clone(),
        route_service.clone(),
        group_service.clone(),
    );

    let sb_svc = SouthboundApiService::new(
        db.clone(),
        cmd_handler.clone(),
        route_service.clone(),
        group_service.clone(),
    );

    // ── gRPC servers ──────────────────────────────────────────────────────────
    // A single drain channel coordinates graceful shutdown of both servers.
    let (drain_tx, drain_rx) = drain::channel();

    cfg.northbound
        .run_server(
            &[ControlPlaneServiceServer::new(nb_svc)],
            drain_rx.clone(),
        )
        .await
        .context("failed to start northbound server")?;

    tracing::info!(
        "Northbound API Service is listening on {}",
        cfg.northbound.endpoint
    );

    cfg.southbound
        .run_server(&[ControllerServiceServer::new(sb_svc)], drain_rx)
        .await
        .context("failed to start southbound server")?;

    tracing::info!(
        "Southbound API Service is listening on {}",
        cfg.southbound.endpoint
    );

    tracing::info!("control plane started");

    // Block until SIGINT or SIGTERM is received.
    slim_signal::shutdown().await;
    tracing::info!("shutdown signal received, draining connections");

    // Stop reconciler workers and wait for in-flight reconciliations to finish.
    route_service.shutdown().await;

    // Signal both gRPC servers to stop and wait for them to finish.
    drain_tx.drain().await;

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
