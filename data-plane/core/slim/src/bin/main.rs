// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{debug, info, span, warn};

use slim::args;
use slim::build_info;
use slim::config;
use slim::runtime;
use slim_config::component::Component;
use slim_config::tls::provider;

fn main() -> anyhow::Result<()> {
    let args = args::Args::parse();

    // If the version flag is set, print the build info and exit
    if args.version() {
        println!("{}", build_info::BUILD_INFO);
        return Ok(());
    }

    // get config file
    let config_file = args.config().context("config file is required")?;

    // create configured components
    let mut config =
        config::ConfigLoader::new(config_file).context("failed to load configuration")?;

    // print build info
    info!(build_info = %build_info::BUILD_INFO);

    // Make sure the crypto provider is initialized at this point
    provider::initialize_crypto_provider();

    // start runtime
    let runtime_config = config.runtime();
    let drain_timeout = runtime_config.drain_timeout();
    let runtime = runtime::build(runtime_config);

    let run_result: Result<_> = runtime.runtime.block_on(async move {
        // tracing subscriber initialization must be called from the runtime
        let tracing_conf = config.tracing();
        let _guard = tracing_conf.setup_tracing_subscriber();

        let root_span = span!(tracing::Level::INFO, "application_lifecycle");
        let _enter = root_span.enter();

        // log the tracing configuration
        debug!(?tracing_conf);

        info!("Runtime started");
        info!(
            telemetry = true,
            monotonic_counter.num_messages_by_type = 1,
            message_type = "subscribe"
        );

        let services = config.services().context("error loading services")?;

        // start services
        for service in services.iter_mut() {
            debug!(service = %service.0, "service starting...");
            service.1.start().await.context("failed to start service")?;
            info!(service = %service.0, "service started");
        }

        // wait for shutdown signal
        tokio::select! {
            _ = slim_signal::shutdown() => {
                debug!("Received shutdown signal");
            }
        }

        // Gracefully stop services with timeout
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
            Ok(result) => {
                result?;
            }
            Err(_) => {
                warn!(
                    timeout = ?drain_timeout,
                    "Service shutdown timed out"
                );
                anyhow::bail!("Service shutdown timed out after {:?}", drain_timeout);
            }
        }
        Ok(())
    });

    run_result?;

    Ok(())
}
