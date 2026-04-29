// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel Manager CTL - CLI client for interacting with the Channel Manager gRPC service.
//!
//! Usage:
//!   cmctl <command> [options]
//!
//! The gRPC connection can be configured either with:
//!   - `--server <address>` for simple insecure connections
//!   - `--client-config <file>` for full configuration (TLS, auth, keepalive, proxy, etc.)
//!     using the same YAML format as the SLIM data-plane ClientConfig
//!
//! Available commands:
//!   list-channels          List all channels
//!   list-participants      List participants in a channel
//!   create-channel         Create a new channel (MLS enabled by default)
//!   delete-channel         Delete a channel
//!   add-participant        Add participant to channel
//!   delete-participant     Remove participant from channel

use std::path::PathBuf;

use agntcy_slim_channel_manager::proto::channel_manager_service_client::ChannelManagerServiceClient;
use agntcy_slim_channel_manager::proto::{
    AddParticipantRequest, CommandResponse, CreateChannelRequest, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use slim_config::grpc::client::ClientConfig;

/// Channel Manager Control Tool
#[derive(Parser)]
#[command(name = "cmctl")]
#[command(about = "Channel Manager Control Tool - interact with the Channel Manager gRPC service")]
struct Args {
    /// Simple gRPC server address (e.g., http://localhost:10356).
    /// Mutually exclusive with --client-config.
    #[arg(long = "server", default_value = "http://localhost:10356")]
    server: String,

    /// Path to a YAML file with full gRPC client configuration (TLS, auth, keepalive, proxy, etc.).
    /// When provided, --server is ignored.
    #[arg(long = "client-config")]
    client_config: Option<PathBuf>,

    /// Command to execute
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create a new channel
    #[command(name = "create-channel")]
    CreateChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Disable MLS for this channel (MLS is enabled by default)
        #[arg(long = "disable-mls", default_value_t = false)]
        disable_mls: bool,
    },

    /// Delete a channel
    #[command(name = "delete-channel")]
    DeleteChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
    },

    /// Add a participant to a channel
    #[command(name = "add-participant")]
    AddParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// Remove a participant from a channel
    #[command(name = "delete-participant")]
    DeleteParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// List all channels
    #[command(name = "list-channels")]
    ListChannels,

    /// List participants in a channel
    #[command(name = "list-participants")]
    ListParticipants {
        /// Channel name (org/namespace/channel)
        channel: String,
    },
}

/// Load a ClientConfig from a YAML file
fn load_client_config(path: &PathBuf) -> Result<ClientConfig> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config: ClientConfig = serde_yaml::from_str(&data)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?;
    Ok(config)
}

/// Create a ChannelManagerServiceClient from either --client-config or --server
async fn create_client(
    args: &Args,
) -> Result<
    ChannelManagerServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<tonic::codegen::StdError> + Send,
            ResponseBody: tonic::codegen::Body<
                Data = tonic::codegen::Bytes,
                Error: Into<tonic::codegen::StdError> + Send,
            > + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
    >,
> {
    // Initialize the crypto provider — needed by ClientConfig::to_channel() for TLS support
    slim_config::tls::provider::initialize_crypto_provider();

    let client_config = if let Some(config_path) = &args.client_config {
        load_client_config(config_path)?
    } else {
        ClientConfig::with_endpoint(&args.server)
    };

    let channel = client_config
        .to_channel()
        .await
        .context("failed to create gRPC channel")?;
    Ok(ChannelManagerServiceClient::new(channel))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut client = create_client(&args).await?;

    match args.command {
        Command::CreateChannel {
            channel,
            disable_mls,
        } => {
            let request = CreateChannelRequest {
                channel_name: channel.clone(),
                mls_enabled: !disable_mls,
            };
            let response = client
                .create_channel(request)
                .await
                .context("failed to create channel")?;
            check_command_response(
                response.into_inner(),
                &format!("Channel {channel} created successfully"),
            )
        }

        Command::DeleteChannel { channel } => {
            let request = DeleteChannelRequest {
                channel_name: channel.clone(),
            };
            let response = client
                .delete_channel(request)
                .await
                .context("failed to delete channel")?;
            check_command_response(
                response.into_inner(),
                &format!("Channel {channel} deleted successfully"),
            )
        }

        Command::AddParticipant {
            channel,
            participant,
        } => {
            let request = AddParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = client
                .add_participant(request)
                .await
                .context("failed to add participant")?;
            check_command_response(
                response.into_inner(),
                &format!("Participant {participant} added to channel {channel}"),
            )
        }

        Command::DeleteParticipant {
            channel,
            participant,
        } => {
            let request = DeleteParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = client
                .delete_participant(request)
                .await
                .context("failed to delete participant")?;
            check_command_response(
                response.into_inner(),
                &format!("Participant {participant} removed from channel {channel}"),
            )
        }

        Command::ListChannels => {
            let response = client
                .list_channels(ListChannelsRequest {})
                .await
                .context("failed to list channels")?;
            let resp = response.into_inner();
            if !resp.success {
                bail!(
                    "{}",
                    resp.error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!("Channels ({}):", resp.channel_name.len());
            for name in &resp.channel_name {
                println!("  - {name}");
            }
            Ok(())
        }

        Command::ListParticipants { channel } => {
            let request = ListParticipantsRequest {
                channel_name: channel.clone(),
            };
            let response = client
                .list_participants(request)
                .await
                .context("failed to list participants")?;
            let resp = response.into_inner();
            if !resp.success {
                bail!(
                    "{}",
                    resp.error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!(
                "Participants in channel {channel} ({}):",
                resp.participant_name.len()
            );
            for name in &resp.participant_name {
                println!("  - {name}");
            }
            Ok(())
        }
    }
}

/// Check a command response, printing success message or returning an error
fn check_command_response(response: CommandResponse, success_msg: &str) -> Result<()> {
    if response.success {
        println!("{success_msg}");
        Ok(())
    } else {
        bail!(
            "{}",
            response
                .error_msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
}
