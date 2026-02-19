// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod build_info;
mod client;
mod commands;
mod config;
mod proto;
mod utils;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use commands::{
    config_cmd::{self, ConfigArgs},
    controller::{self, ControllerArgs},
    node::{self, NodeArgs},
    slim_cmd::{self, SlimArgs},
    version,
};
use config::{ResolvedOpts, load_config};

/// SLIM control CLI
#[derive(Parser)]
#[command(name = "slimctl", about = "SLIM control CLI")]
struct Cli {
    #[command(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

/// Global options applied to all commands
#[derive(Args)]
struct GlobalOpts {
    /// Basic auth credentials (username:password)
    #[arg(
        short = 'b',
        long,
        global = true,
        env = "SLIMCTL_COMMON_OPTS_BASIC_AUTH_CREDS"
    )]
    basic_auth_creds: Option<String>,

    /// SLIM gRPC control API endpoint (host:port)
    #[arg(short = 's', long, global = true, env = "SLIMCTL_COMMON_OPTS_SERVER")]
    server: Option<String>,

    /// gRPC request timeout (e.g. 15s, 1m)
    #[arg(long, global = true, env = "SLIMCTL_COMMON_OPTS_TIMEOUT")]
    timeout: Option<String>,

    /// Disable TLS (plain HTTP/2)
    #[arg(long, global = true, env = "SLIMCTL_COMMON_OPTS_TLS_INSECURE")]
    tls_insecure: Option<bool>,

    /// Use TLS but skip server certificate verification
    #[arg(
        long,
        global = true,
        env = "SLIMCTL_COMMON_OPTS_TLS_INSECURE_SKIP_VERIFY"
    )]
    tls_insecure_skip_verify: Option<bool>,

    /// Path to TLS CA certificate
    #[arg(long, global = true, env = "SLIMCTL_COMMON_OPTS_TLS_CA_FILE")]
    tls_ca_file: Option<String>,

    /// Path to client TLS certificate
    #[arg(long, global = true, env = "SLIMCTL_COMMON_OPTS_TLS_CERT_FILE")]
    tls_cert_file: Option<String>,

    /// Path to client TLS key
    #[arg(long, global = true, env = "SLIMCTL_COMMON_OPTS_TLS_KEY_FILE")]
    tls_key_file: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Print version information
    Version,

    /// Manage slimctl configuration
    Config(ConfigArgs),

    /// Commands to interact with SLIM nodes directly
    #[command(aliases = ["n", "instance", "i"])]
    Node(NodeArgs),

    /// Commands to interact with the SLIM Control Plane
    #[command(aliases = ["c", "ctrl"])]
    Controller(ControllerArgs),

    /// Commands for managing a local SLIM instance
    #[command(alias = "s")]
    Slim(SlimArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Load config file, then overlay with CLI/env opts
    let file_config = load_config()?;
    let opts = ResolvedOpts::resolve(
        &file_config,
        cli.global.server.as_deref(),
        cli.global.timeout.as_deref(),
        cli.global.tls_insecure,
        cli.global.tls_insecure_skip_verify,
        cli.global.tls_ca_file.as_deref(),
        cli.global.tls_cert_file.as_deref(),
        cli.global.tls_key_file.as_deref(),
        cli.global.basic_auth_creds.as_deref(),
    )?;

    match cli.command {
        Commands::Version => {
            version::run();
        }
        Commands::Config(args) => {
            config_cmd::run(&args).await?;
        }
        Commands::Node(args) => {
            node::run(&args, &opts).await?;
        }
        Commands::Controller(args) => {
            controller::run(&args, &opts).await?;
        }
        Commands::Slim(args) => {
            slim_cmd::run(&args).await?;
        }
    }

    Ok(())
}
