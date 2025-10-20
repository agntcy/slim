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
    let mut config = config::load_config(config_file).expect("failed to load configuration");

    // print build info
    info!("{}", build_info::BUILD_INFO);

    // Make sure the crypto provider is initialized at this point
    provider::initialize_crypto_provider();

    // start runtime
    let runtime = runtime::build(&config.runtime).expect("failed to build runtime");
    runtime.runtime.block_on(async move {
        // tracing subscriber initialization must be called from the runtime
        let _guard = config.tracing.setup_tracing_subscriber();

        let root_span = span!(tracing::Level::INFO, "application_lifecycle");
        let _enter = root_span.enter();

        // log the tracing configuration
        debug!(?config.tracing);

        info!("Runtime started");
        info!(
            telemetry = true,
            monotonic_counter.num_messages_by_type = 1,
            message_type = "subscribe"
        );

        // start services
        for service in config.services.iter_mut() {
            info!("Starting service: {}", service.0);
            service.1.start().await.expect("failed to start service")
        }

        // wait for shutdown signal
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
            }
        }

        // Send a drain signal to all services
        for svc in config.services {
            // consume the service and get the drain signal
            let signal = svc.1.signal();

            match time::timeout(runtime.config.drain_timeout(), signal.drain()).await {
                Ok(()) => {}
                Err(_) => panic!("timeout waiting for drain for service {}", svc.0),
            }
        }
    });
}
