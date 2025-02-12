// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Optional name to operate on
    name: Option<String>,

    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    #[clap(long, env, required = true)]
    config: String,
}

impl Args {
    // Temporary disable warnings
    #[allow(dead_code)]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn config(&self) -> &String {
        &self.config
    }
}
