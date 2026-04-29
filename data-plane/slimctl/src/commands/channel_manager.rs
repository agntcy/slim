// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::client::get_channel_manager_client;
use crate::proto::channel_manager::proto::v1::{
    AddParticipantRequest, CommandResponse, CreateChannelRequest, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
};
use crate::rpc;
use slim_config::grpc::client::ClientConfig;

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

fn check_command_response(response: CommandResponse, success_msg: &str) -> Result<()> {
    if response.success {
        println!("{success_msg}");
        Ok(())
    } else {
        let err = response
            .error_msg
            .unwrap_or_else(|| "unknown error".to_string());
        bail!("Command failed: {err}")
    }
}

pub async fn run(args: &ChannelManagerArgs, opts: &ClientConfig) -> Result<()> {
    let mut client = get_channel_manager_client(opts).await?;

    match &args.command {
        ChannelManagerCommand::CreateChannel {
            channel,
            disable_mls,
        } => {
            let request = CreateChannelRequest {
                channel_name: channel.clone(),
                mls_enabled: !disable_mls,
            };
            let response = rpc!(client, create_channel, request);
            check_command_response(response, &format!("Channel {channel} created successfully"))
        }

        ChannelManagerCommand::DeleteChannel { channel } => {
            let request = DeleteChannelRequest {
                channel_name: channel.clone(),
            };
            let response = rpc!(client, delete_channel, request);
            check_command_response(response, &format!("Channel {channel} deleted successfully"))
        }

        ChannelManagerCommand::AddParticipant {
            channel,
            participant,
        } => {
            let request = AddParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = rpc!(client, add_participant, request);
            check_command_response(
                response,
                &format!("Participant {participant} added to channel {channel}"),
            )
        }

        ChannelManagerCommand::DeleteParticipant {
            channel,
            participant,
        } => {
            let request = DeleteParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = rpc!(client, delete_participant, request);
            check_command_response(
                response,
                &format!("Participant {participant} removed from channel {channel}"),
            )
        }

        ChannelManagerCommand::ListChannels => {
            let response = rpc!(client, list_channels, ListChannelsRequest {});
            if !response.success {
                bail!(
                    "{}",
                    response
                        .error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!("Channels ({}):", response.channel_name.len());
            for name in &response.channel_name {
                println!("  - {name}");
            }
            Ok(())
        }

        ChannelManagerCommand::ListParticipants { channel } => {
            let request = ListParticipantsRequest {
                channel_name: channel.clone(),
            };
            let response = rpc!(client, list_participants, request);
            if !response.success {
                bail!(
                    "{}",
                    response
                        .error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!(
                "Participants in channel {channel} ({}):",
                response.participant_name.len()
            );
            for name in &response.participant_name {
                println!("  - {name}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_stream::wrappers::TcpListenerStream;

    use crate::proto::channel_manager::proto::v1::{
        AddParticipantRequest, CommandResponse, CreateChannelRequest, DeleteChannelRequest,
        DeleteParticipantRequest, ListChannelsRequest, ListChannelsResponse,
        ListParticipantsRequest, ListParticipantsResponse,
        channel_manager_service_server::{ChannelManagerService, ChannelManagerServiceServer},
    };
    use slim_config::grpc::client::ClientConfig;

    use super::*;

    // ── check_command_response unit tests ────────────────────────────────────

    #[test]
    fn check_command_response_success() {
        let resp = CommandResponse {
            success: true,
            error_msg: None,
        };
        assert!(check_command_response(resp, "ok").is_ok());
    }

    #[test]
    fn check_command_response_failure_with_message() {
        let resp = CommandResponse {
            success: false,
            error_msg: Some("bad channel".to_string()),
        };
        let err = check_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("bad channel"));
    }

    #[test]
    fn check_command_response_failure_without_message() {
        let resp = CommandResponse {
            success: false,
            error_msg: None,
        };
        let err = check_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("unknown error"));
    }

    // ── mock gRPC servers ───────────────────────────────────────────────────

    struct MockChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for MockChannelManagerSvc {
        async fn create_channel(
            &self,
            _request: tonic::Request<CreateChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn delete_channel(
            &self,
            _request: tonic::Request<DeleteChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn add_participant(
            &self,
            _request: tonic::Request<AddParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn delete_participant(
            &self,
            _request: tonic::Request<DeleteParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn list_channels(
            &self,
            _request: tonic::Request<ListChannelsRequest>,
        ) -> Result<tonic::Response<ListChannelsResponse>, tonic::Status> {
            Ok(tonic::Response::new(ListChannelsResponse {
                success: true,
                error_msg: None,
                channel_name: vec!["org/ns/chan1".to_string(), "org/ns/chan2".to_string()],
            }))
        }

        async fn list_participants(
            &self,
            _request: tonic::Request<ListParticipantsRequest>,
        ) -> Result<tonic::Response<ListParticipantsResponse>, tonic::Status> {
            Ok(tonic::Response::new(ListParticipantsResponse {
                success: true,
                error_msg: None,
                participant_name: vec!["org/ns/app1".to_string()],
            }))
        }
    }

    struct ErrorChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for ErrorChannelManagerSvc {
        async fn create_channel(
            &self,
            _request: tonic::Request<CreateChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }

        async fn delete_channel(
            &self,
            _request: tonic::Request<DeleteChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }

        async fn add_participant(
            &self,
            _request: tonic::Request<AddParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }

        async fn delete_participant(
            &self,
            _request: tonic::Request<DeleteParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }

        async fn list_channels(
            &self,
            _request: tonic::Request<ListChannelsRequest>,
        ) -> Result<tonic::Response<ListChannelsResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }

        async fn list_participants(
            &self,
            _request: tonic::Request<ListParticipantsRequest>,
        ) -> Result<tonic::Response<ListParticipantsResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }
    }

    struct NackChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for NackChannelManagerSvc {
        async fn create_channel(
            &self,
            _request: tonic::Request<CreateChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: false,
                error_msg: Some("nack".to_string()),
            }))
        }

        async fn delete_channel(
            &self,
            _request: tonic::Request<DeleteChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: false,
                error_msg: Some("nack".to_string()),
            }))
        }

        async fn add_participant(
            &self,
            _request: tonic::Request<AddParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: false,
                error_msg: Some("nack".to_string()),
            }))
        }

        async fn delete_participant(
            &self,
            _request: tonic::Request<DeleteParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Ok(tonic::Response::new(CommandResponse {
                success: false,
                error_msg: Some("nack".to_string()),
            }))
        }

        async fn list_channels(
            &self,
            _request: tonic::Request<ListChannelsRequest>,
        ) -> Result<tonic::Response<ListChannelsResponse>, tonic::Status> {
            Ok(tonic::Response::new(ListChannelsResponse {
                success: false,
                error_msg: Some("list denied".to_string()),
                channel_name: vec![],
            }))
        }

        async fn list_participants(
            &self,
            _request: tonic::Request<ListParticipantsRequest>,
        ) -> Result<tonic::Response<ListParticipantsResponse>, tonic::Status> {
            Ok(tonic::Response::new(ListParticipantsResponse {
                success: false,
                error_msg: Some("list denied".to_string()),
                participant_name: vec![],
            }))
        }
    }

    async fn spawn_svc<S>(svc: S) -> String
    where
        S: ChannelManagerService,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpListenerStream::new(listener);
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(ChannelManagerServiceServer::new(svc))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        format!("{}:{}", addr.ip(), addr.port())
    }

    fn make_opts(addr: &str) -> ClientConfig {
        slim_config::grpc::client::ClientConfig::with_endpoint(&format!("http://{}", addr))
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure())
            .with_request_timeout(Duration::from_secs(5))
    }

    // ── happy-path tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_channel_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::CreateChannel {
                channel: "org/ns/chan".to_string(),
                disable_mls: false,
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn create_channel_disable_mls_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::CreateChannel {
                channel: "org/ns/chan".to_string(),
                disable_mls: true,
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn delete_channel_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "org/ns/chan".to_string(),
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn add_participant_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::AddParticipant {
                channel: "org/ns/chan".to_string(),
                participant: "org/ns/app".to_string(),
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn delete_participant_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteParticipant {
                channel: "org/ns/chan".to_string(),
                participant: "org/ns/app".to_string(),
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn list_channels_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListChannels,
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    #[tokio::test]
    async fn list_participants_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListParticipants {
                channel: "org/ns/chan".to_string(),
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
    }

    // ── gRPC-level error tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn create_channel_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::CreateChannel {
                channel: "c".to_string(),
                disable_mls: false,
            },
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn delete_channel_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "c".to_string(),
            },
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn add_participant_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::AddParticipant {
                channel: "c".to_string(),
                participant: "p".to_string(),
            },
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn delete_participant_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteParticipant {
                channel: "c".to_string(),
                participant: "p".to_string(),
            },
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn list_channels_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListChannels,
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    #[tokio::test]
    async fn list_participants_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListParticipants {
                channel: "c".to_string(),
            },
        };
        assert!(run(&args, &make_opts(&addr)).await.is_err());
    }

    // ── negative-ACK tests (success = false) ────────────────────────────────

    #[tokio::test]
    async fn create_channel_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::CreateChannel {
                channel: "c".to_string(),
                disable_mls: false,
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("nack"));
    }

    #[tokio::test]
    async fn delete_channel_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "c".to_string(),
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("nack"));
    }

    #[tokio::test]
    async fn add_participant_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::AddParticipant {
                channel: "c".to_string(),
                participant: "p".to_string(),
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("nack"));
    }

    #[tokio::test]
    async fn delete_participant_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteParticipant {
                channel: "c".to_string(),
                participant: "p".to_string(),
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("nack"));
    }

    #[tokio::test]
    async fn list_channels_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListChannels,
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("list denied"));
    }

    #[tokio::test]
    async fn list_participants_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::ListParticipants {
                channel: "c".to_string(),
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("list denied"));
    }
}
