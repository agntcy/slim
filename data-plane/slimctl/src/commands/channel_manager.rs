// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use rand::Rng;

use crate::client::get_channel_manager_client;
use crate::config::ResolvedOpts;
use crate::proto::channel_manager::proto::v1::{
    AddParticipantRequest, ControlRequest, ControlResponse, CreateChannelRequest,
    DeleteChannelRequest, DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
    control_request::Payload,
    control_response::Payload as ResponsePayload,
};
use crate::rpc;

#[derive(Args)]
pub struct ChannelManagerArgs {
    #[command(subcommand)]
    pub command: ChannelManagerCommand,
}

#[derive(Subcommand)]
pub enum ChannelManagerCommand {
    /// Create a new channel
    #[command(name = "create-channel", visible_alias = "cc")]
    CreateChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Disable MLS for this channel (MLS is enabled by default)
        #[arg(long = "disable-mls", default_value_t = false)]
        disable_mls: bool,
    },

    /// Delete a channel
    #[command(name = "delete-channel", visible_alias = "dc")]
    DeleteChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
    },

    /// Add a participant to a channel
    #[command(name = "add-participant", visible_alias = "ap")]
    AddParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// Remove a participant from a channel
    #[command(name = "delete-participant", visible_alias = "dp")]
    DeleteParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// List all channels
    #[command(name = "list-channels", visible_alias = "lc")]
    ListChannels,

    /// List participants in a channel
    #[command(name = "list-participants", visible_alias = "lp")]
    ListParticipants {
        /// Channel name (org/namespace/channel)
        channel: String,
    },
}

fn generate_msg_id() -> u64 {
    rand::rng().random()
}

fn handle_command_response(response: ControlResponse, success_msg: &str) -> Result<()> {
    match response.payload {
        Some(ResponsePayload::CommandResponse(cmd_resp)) => {
            if cmd_resp.success {
                println!("{success_msg}");
                Ok(())
            } else {
                let err = cmd_resp
                    .error_msg
                    .unwrap_or_else(|| "unknown error".to_string());
                bail!("Command failed: {err}")
            }
        }
        _ => bail!("Unexpected response type"),
    }
}

pub async fn run(args: &ChannelManagerArgs, opts: &ResolvedOpts) -> Result<()> {
    let mut client = get_channel_manager_client(opts).await?;

    match &args.command {
        ChannelManagerCommand::CreateChannel {
            channel,
            disable_mls,
        } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::CreateChannelRequest(CreateChannelRequest {
                    channel_name: channel.clone(),
                    mls_enabled: !disable_mls,
                })),
            };
            let response = rpc!(client, command, request, opts);
            handle_command_response(response, &format!("Channel {channel} created successfully"))
        }

        ChannelManagerCommand::DeleteChannel { channel } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::DeleteChannelRequest(DeleteChannelRequest {
                    channel_name: channel.clone(),
                })),
            };
            let response = rpc!(client, command, request, opts);
            handle_command_response(response, &format!("Channel {channel} deleted successfully"))
        }

        ChannelManagerCommand::AddParticipant {
            channel,
            participant,
        } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::AddParticipantRequest(AddParticipantRequest {
                    channel_name: channel.clone(),
                    participant_name: participant.clone(),
                })),
            };
            let response = rpc!(client, command, request, opts);
            handle_command_response(
                response,
                &format!("Participant {participant} added to channel {channel}"),
            )
        }

        ChannelManagerCommand::DeleteParticipant {
            channel,
            participant,
        } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::DeleteParticipantRequest(
                    DeleteParticipantRequest {
                        channel_name: channel.clone(),
                        participant_name: participant.clone(),
                    },
                )),
            };
            let response = rpc!(client, command, request, opts);
            handle_command_response(
                response,
                &format!("Participant {participant} removed from channel {channel}"),
            )
        }

        ChannelManagerCommand::ListChannels => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::ListChannelsRequest(ListChannelsRequest {})),
            };
            let response = rpc!(client, command, request, opts);
            match response.payload {
                Some(ResponsePayload::ListChannelsResponse(list)) => {
                    println!("Channels ({}):", list.channel_name.len());
                    for name in &list.channel_name {
                        println!("  - {name}");
                    }
                    Ok(())
                }
                Some(ResponsePayload::CommandResponse(cmd_resp)) if !cmd_resp.success => {
                    let err = cmd_resp
                        .error_msg
                        .unwrap_or_else(|| "unknown error".to_string());
                    bail!("Command failed: {err}")
                }
                _ => bail!("Unexpected response type"),
            }
        }

        ChannelManagerCommand::ListParticipants { channel } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::ListParticipantsRequest(
                    ListParticipantsRequest {
                        channel_name: channel.clone(),
                    },
                )),
            };
            let response = rpc!(client, command, request, opts);
            match response.payload {
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
                    let err = cmd_resp
                        .error_msg
                        .unwrap_or_else(|| "unknown error".to_string());
                    bail!("Command failed: {err}")
                }
                _ => bail!("Unexpected response type"),
            }
        }
    }
}
