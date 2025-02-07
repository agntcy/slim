// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Path to the config file
    #[arg(short, long, value_name = "FILE")]
    #[clap(long, env, required = true)]
    config: String,

    /// Set the topic to subscribe to.
    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "LOCAL_AGENT")]
    local_agent: String,

    /// Set the topic to subscribe to.
    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "REMOTE_AGENT")]
    remote_agent: String,

    /// Set the message to publish. If not set, the program will subscribe to the topic.
    #[clap(long, env)]
    #[arg(short, long, value_name = "MESSAGE")]
    message: Option<String>,
}

impl Args {
    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn local_agent(&self) -> &str {
        &self.local_agent
    }

    pub fn remote_agent(&self) -> &str {
        &self.remote_agent
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}
