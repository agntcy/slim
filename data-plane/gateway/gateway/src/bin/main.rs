// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tokio::time;
use tracing::info;

use agp_config::component::Component;
use agp_gw::args;
use agp_gw::build_info;
use agp_gw::config;
use agp_gw::runtime;

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
    let config = config::load_config(config_file).expect("failed to load configuration");

    // print build info
    info!("{}", build_info::BUILD_INFO);

    // start runtime
    let runtime = runtime::build(&config.runtime).expect("failed to build runtime");
    runtime.runtime.block_on(async move {
        info!("Runtime started");

        // start services
        for service in config.services.iter() {
            info!("Starting service: {}", service.0);
            service.1.start().await.expect("failed to start service")
        }

        // wait for shutdown signal
        tokio::select! {
            _ = agp_signal::shutdown() => {
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
