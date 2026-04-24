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
use std::process;

use agntcy_slim_channel_manager::proto::channel_manager_service_client::ChannelManagerServiceClient;
use agntcy_slim_channel_manager::proto::{
    AddParticipantRequest, ControlRequest, CreateChannelRequest, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
    control_request::Payload,
    control_response::Payload as ResponsePayload,
};

use clap::Parser;
use rand::Rng;
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

/// Generate a random message ID
fn generate_msg_id() -> u64 {
    rand::rng().random()
}

/// Load a ClientConfig from a YAML file
fn load_client_config(path: &PathBuf) -> Result<ClientConfig, Box<dyn std::error::Error>> {
    let data = std::fs::read_to_string(path)?;
    let config: ClientConfig = serde_yaml::from_str(&data)?;
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
    Box<dyn std::error::Error>,
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
        .map_err(|e| format!("failed to create gRPC channel: {e}"))?;
    Ok(ChannelManagerServiceClient::new(channel))
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let mut client = match create_client(&args).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create client: {e}");
            process::exit(1);
        }
    };

    let result = match args.command {
        Command::CreateChannel { channel, disable_mls } => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::CreateChannelRequest(CreateChannelRequest {
                    channel_name: channel.clone(),
                    mls_enabled: !disable_mls,
                })),
            };
            match client.command(request).await {
                Ok(response) => {
                    handle_command_response(response.into_inner(), &format!("Channel {channel} created successfully"))
                }
                Err(e) => {
                    eprintln!("Failed to create channel: {e}");
                    process::exit(1);
                }
            }
        }

        Command::DeleteChannel { channel } => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::DeleteChannelRequest(DeleteChannelRequest {
                    channel_name: channel.clone(),
                })),
            };
            match client.command(request).await {
                Ok(response) => {
                    handle_command_response(response.into_inner(), &format!("Channel {channel} deleted successfully"))
                }
                Err(e) => {
                    eprintln!("Failed to delete channel: {e}");
                    process::exit(1);
                }
            }
        }

        Command::AddParticipant {
            channel,
            participant,
        } => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::AddParticipantRequest(AddParticipantRequest {
                    channel_name: channel.clone(),
                    participant_name: participant.clone(),
                })),
            };
            match client.command(request).await {
                Ok(response) => handle_command_response(
                    response.into_inner(),
                    &format!("Participant {participant} added to channel {channel}"),
                ),
                Err(e) => {
                    eprintln!("Failed to add participant: {e}");
                    process::exit(1);
                }
            }
        }

        Command::DeleteParticipant {
            channel,
            participant,
        } => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::DeleteParticipantRequest(
                    DeleteParticipantRequest {
                        channel_name: channel.clone(),
                        participant_name: participant.clone(),
                    },
                )),
            };
            match client.command(request).await {
                Ok(response) => handle_command_response(
                    response.into_inner(),
                    &format!("Participant {participant} removed from channel {channel}"),
                ),
                Err(e) => {
                    eprintln!("Failed to delete participant: {e}");
                    process::exit(1);
                }
            }
        }

        Command::ListChannels => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::ListChannelsRequest(ListChannelsRequest {})),
            };
            match client.command(request).await {
                Ok(response) => {
                    let resp = response.into_inner();
                    match resp.payload {
                        Some(ResponsePayload::ListChannelsResponse(list)) => {
                            println!("Channels ({}):", list.channel_name.len());
                            for name in &list.channel_name {
                                println!("  - {name}");
                            }
                            Ok(())
                        }
                        Some(ResponsePayload::CommandResponse(cmd_resp)) if !cmd_resp.success => {
                            let err = cmd_resp.error_msg.unwrap_or_else(|| "unknown error".to_string());
                            Err(format!("Command failed: {err}"))
                        }
                        _ => Err("Unexpected response type".to_string()),
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list channels: {e}");
                    process::exit(1);
                }
            }
        }

        Command::ListParticipants { channel } => {
            let msg_id = generate_msg_id();
            let request = ControlRequest {
                msg_id,
                payload: Some(Payload::ListParticipantsRequest(
                    ListParticipantsRequest {
                        channel_name: channel.clone(),
                    },
                )),
            };
            match client.command(request).await {
                Ok(response) => {
                    let resp = response.into_inner();
                    match resp.payload {
                        Some(ResponsePayload::ListParticipantsResponse(list)) => {
                            println!(
                                "Participants in channel {channel} ({}):",
                                list.participant_name.len()
                            );
                            for name in &list.participant_name {
                                println!("  - {name}");
                            }
                            Ok(())
                        }
                        Some(ResponsePayload::CommandResponse(cmd_resp)) if !cmd_resp.success => {
                            let err = cmd_resp.error_msg.unwrap_or_else(|| "unknown error".to_string());
                            Err(format!("Command failed: {err}"))
                        }
                        _ => Err("Unexpected response type".to_string()),
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list participants: {e}");
                    process::exit(1);
                }
            }
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

/// Handle a command response, printing success or error message
fn handle_command_response(
    response: agntcy_slim_channel_manager::proto::ControlResponse,
    success_msg: &str,
) -> Result<(), String> {
    match response.payload {
        Some(ResponsePayload::CommandResponse(cmd_resp)) => {
            if cmd_resp.success {
                println!("{success_msg}");
                Ok(())
            } else {
                let err = cmd_resp.error_msg.unwrap_or_else(|| "unknown error".to_string());
                Err(format!("Command failed: {err}"))
            }
        }
        _ => Err("Unexpected response type".to_string()),
    }
}
