// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use tracing::{debug, info, span, warn};

use crate::build_info;
use crate::config::ConfigLoader;
use crate::runtime;
use slim_config::component::Component;
use slim_config::tls::provider;

/// Async body: tracing setup, service lifecycle, graceful shutdown.
/// Assumes the crypto provider has already been initialized and the caller
/// provides a Tokio runtime.
pub async fn run_services(mut config: ConfigLoader) -> Result<()> {
    // Setup tracing
    let tracing_conf = config.tracing().context("invalid tracing configuration")?;
    let _guard = tracing_conf.setup_tracing_subscriber();

    let root_span = span!(tracing::Level::INFO, "application_lifecycle");
    let _enter = root_span.enter();

    debug!(?tracing_conf);
    info!(build_info = %build_info::BUILD_INFO);
    info!(
        telemetry = true,
        monotonic_counter.num_messages_by_type = 1,
        message_type = "subscribe"
    );

    // Read drain timeout from runtime config
    let drain_timeout = config
        .runtime()
        .context("invalid runtime configuration")?
        .drain_timeout();

    // Load services
    let services = config.services().context("error loading services")?;

    // Start services
    for service in services.iter_mut() {
        debug!(service = %service.0, "service starting...");
        service.1.start().await.context("failed to start service")?;
        info!(service = %service.0, "service started");
    }

    // Wait for shutdown signal
    slim_signal::shutdown().await;
    debug!("Received shutdown signal");

    // Gracefully stop services within the drain timeout
    let shutdown_all = async {
        for service in services.iter_mut() {
            info!(service = %service.0, "stopping service");
            service
                .1
                .shutdown()
                .await
                .context("failed to stop service")?;
        }
        Ok::<(), anyhow::Error>(())
    };

    match tokio::time::timeout(drain_timeout, shutdown_all).await {
        Ok(result) => result?,
        Err(_) => {
            warn!(timeout = ?drain_timeout, "Service shutdown timed out");
            anyhow::bail!("Service shutdown timed out after {:?}", drain_timeout);
        }
    }

    Ok(())
}

/// Load config from `config_file`, initialize crypto, build a Tokio runtime
/// as specified by the `runtime:` section, start all services, and block
/// until a shutdown signal is received.
///
/// This is a **synchronous** blocking call.  Callers already inside an async
/// context (e.g. a CLI subcommand) should use
/// `tokio::task::spawn_blocking(|| runner::run(path))`.
pub fn run(config_file: &str) -> Result<()> {
    let mut config = ConfigLoader::new(config_file).context("failed to load configuration")?;

    provider::initialize_crypto_provider();

    let slim_runtime = runtime::build(config.runtime().context("invalid runtime configuration")?);
    slim_runtime.runtime.block_on(run_services(config))
}
