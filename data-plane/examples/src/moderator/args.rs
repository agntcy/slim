// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Path to the config file
    #[arg(short, long, value_name = "FILE")]
    #[clap(long, env, required = true)]
    config: String,

    /// Moderator name (e.g., "moderator")
    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "NAME")]
    name: String,

    /// Enable MLS
    #[clap(long, env)]
    #[arg(long)]
    mls: bool,
}

impl Args {
    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mls(&self) -> bool {
        self.mls
    }
}
