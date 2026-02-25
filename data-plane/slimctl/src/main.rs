// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod build_info;
mod cli;
mod client;
mod commands;
mod config;
mod defaults;
mod proto;
mod utils;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let parsed = cli::Cli::parse();
    cli::run(parsed).await
}
