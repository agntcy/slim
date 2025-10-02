// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, value_name = "FILE")]
    #[clap(long, env, required = true)]
    config: String,

    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "LOCAL_NAME")]
    local_name: String,

    #[clap(long, env, required = false)]
    #[arg(short, long, value_name = "MESSAGE")]
    message: Option<String>,

    #[clap(long, env, required = false, default_value = "secret")]
    #[arg(short, long, value_name = "SECRET")]
    secret: String,
}

impl Args {
    pub fn config(&self) -> &str {
        &self.config
    }

    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    pub fn message(&self) -> &Option<String> {
        &self.message
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }
}
