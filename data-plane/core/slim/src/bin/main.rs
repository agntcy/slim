// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use clap::Parser;

use slim::args;
use slim::build_info;
use slim::runner;

fn main() -> anyhow::Result<()> {
    let args = args::Args::parse();

    // If the version flag is set, print the build info and exit
    if args.version() {
        println!("{}", build_info::BUILD_INFO);
        return Ok(());
    }

    // get config file
    let config_file = args.config().context("config file is required")?;

    runner::run(config_file)
}
