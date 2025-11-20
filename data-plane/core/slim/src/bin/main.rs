// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use slim_config::tls::provider;
use tokio::time;
use tracing::{debug, info, span};

use slim::args;
use slim::build_info;
use slim::config;
use slim::runtime;
use slim_config::component::Component;

fn main() {
    let args = args::Args::parse();

    // If the version flag is set, print the build info and exit
    if args.version() {
        println!("{}", build_info::BUILD_INFO);
        return;
    }

    // get config file
    let config_file = args.config().expect("config file is required");

    // create configured components
    let mut config = config::ConfigLoader::new(config_file).expect("failed to load configuration");

    // print build info
    info!("{}", build_info::BUILD_INFO);

    // Make sure the crypto provider is initialized at this point
    provider::initialize_crypto_provider();

    // start runtime
    let runtime_config = config.runtime();
    let runtime = runtime::build(runtime_config).expect("failed to build runtime");

    runtime.runtime.block_on(async move {
        // tracing subscriber initialization
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

        let services = config.services().expect("error loading services");

        // start services
        for service in services.iter_mut() {
            info!("Starting service: {}", service.0);
            service.1.start().await.expect("failed to start service")
        }

        // wait for shutdown signal
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
            }
        }

        // Get all signals from services
        let signals = services
            .iter_mut()
            .filter_map(|svc| svc.1.signal().map(|signal| (svc.0.clone(), signal)))
            .collect::<Vec<_>>();

        // Drop confis to release any resources before draining
        drop(config);

        // Send a drain signal to all services
        for (id, signal) in signals {
            info!("Draining service: {}", id);

            match time::timeout(runtime.config.drain_timeout(), signal.drain()).await {
                Ok(()) => {}
                Err(_) => panic!("timeout waiting for drain for service {}", id),
            }
        }
    });
}
