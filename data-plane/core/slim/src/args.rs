// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(about, long_about = None)]
pub struct Args {
    /// Sets a custom config file
    #[arg(short, long, required = true, value_name = "FILE", group = "1")]
    #[clap(long, env)]
    config: Option<String>,

    /// Print version
    #[clap(short, long, action, required = true, group = "1")]
    version: bool,
}

impl Args {
    pub fn config(&self) -> Option<&str> {
        self.config.as_deref()
    }

    pub fn version(&self) -> bool {
        self.version
    }
}
