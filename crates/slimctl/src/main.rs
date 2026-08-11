// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod build_info;
mod cli;
mod client;
mod commands;
mod config;
mod proto;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let parsed = cli::Cli::parse();
    cli::run(parsed).await.map_err(annotate_auth_failure)
}

/// Append a re-login hint when the failure was the server rejecting our
/// credentials.  Tokens written by `slimctl login` expire, and a bare
/// "Unauthenticated" gives the user nothing to act on.
fn annotate_auth_failure(err: anyhow::Error) -> anyhow::Error {
    let is_auth = err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<tonic::Status>())
        .any(config::is_auth_failure);
    if is_auth {
        err.context(config::auth_failure_hint())
    } else {
        err
    }
}
