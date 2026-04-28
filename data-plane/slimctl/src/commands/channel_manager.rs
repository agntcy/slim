// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use rand::Rng;

use crate::client::get_channel_manager_client;
use crate::proto::channel_manager::proto::v1::{
    self as cm_proto, ControlRequest, ControlResponse, control_request::Payload,
    control_response::Payload as ResponsePayload,
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

pub async fn run(args: &ChannelManagerArgs, opts: &ClientConfig) -> Result<()> {
    let mut client = get_channel_manager_client(opts).await?;

    match &args.command {
        ChannelManagerCommand::CreateChannel {
            channel,
            disable_mls,
        } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::CreateChannel(cm_proto::CreateChannel {
                    channel_name: channel.clone(),
                    mls_enabled: !disable_mls,
                })),
            };
            let response = rpc!(client, command, request);
            handle_command_response(response, &format!("Channel {channel} created successfully"))
        }

        ChannelManagerCommand::DeleteChannel { channel } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::DeleteChannel(cm_proto::DeleteChannel {
                    channel_name: channel.clone(),
                })),
            };
            let response = rpc!(client, command, request);
            handle_command_response(response, &format!("Channel {channel} deleted successfully"))
        }

        ChannelManagerCommand::AddParticipant {
            channel,
            participant,
        } => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::AddParticipant(cm_proto::AddParticipant {
                    channel_name: channel.clone(),
                    participant_name: participant.clone(),
                })),
            };
            let response = rpc!(client, command, request);
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
                payload: Some(Payload::DeleteParticipant(cm_proto::DeleteParticipant {
                    channel_name: channel.clone(),
                    participant_name: participant.clone(),
                })),
            };
            let response = rpc!(client, command, request);
            handle_command_response(
                response,
                &format!("Participant {participant} removed from channel {channel}"),
            )
        }

        ChannelManagerCommand::ListChannels => {
            let request = ControlRequest {
                msg_id: generate_msg_id(),
                payload: Some(Payload::ListChannels(cm_proto::ListChannels {})),
            };
            let response = rpc!(client, command, request);
            match response.payload {
                Some(ResponsePayload::ChannelsList(list)) => {
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
                payload: Some(Payload::ListParticipants(cm_proto::ListParticipants {
                    channel_name: channel.clone(),
                })),
            };
            let response = rpc!(client, command, request);
            match response.payload {
                Some(ResponsePayload::ParticipantsList(list)) => {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_stream::wrappers::TcpListenerStream;

    use crate::proto::channel_manager::proto::v1::{
        ChannelsList, CommandResponse, ControlRequest, ControlResponse, ParticipantsList,
        channel_manager_service_server::{ChannelManagerService, ChannelManagerServiceServer},
        control_request::Payload as RequestPayload,
        control_response::Payload as RespPayload,
    };
    use slim_config::grpc::client::ClientConfig;

    use super::*;

    // ── handle_command_response unit tests ───────────────────────────────────

    #[test]
    fn handle_command_response_success() {
        let resp = ControlResponse {
            msg_id: 1,
            payload: Some(RespPayload::CommandResponse(CommandResponse {
                msg_id: 1,
                success: true,
                error_msg: None,
            })),
        };
        assert!(handle_command_response(resp, "ok").is_ok());
    }

    #[test]
    fn handle_command_response_failure_with_message() {
        let resp = ControlResponse {
            msg_id: 1,
            payload: Some(RespPayload::CommandResponse(CommandResponse {
                msg_id: 1,
                success: false,
                error_msg: Some("bad channel".to_string()),
            })),
        };
        let err = handle_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("bad channel"));
    }

    #[test]
    fn handle_command_response_failure_without_message() {
        let resp = ControlResponse {
            msg_id: 1,
            payload: Some(RespPayload::CommandResponse(CommandResponse {
                msg_id: 1,
                success: false,
                error_msg: None,
            })),
        };
        let err = handle_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("unknown error"));
    }

    #[test]
    fn handle_command_response_unexpected_payload() {
        let resp = ControlResponse {
            msg_id: 1,
            payload: Some(RespPayload::ChannelsList(ChannelsList {
                msg_id: 1,
                channel_name: vec![],
            })),
        };
        let err = handle_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("Unexpected response type"));
    }

    #[test]
    fn handle_command_response_no_payload() {
        let resp = ControlResponse {
            msg_id: 1,
            payload: None,
        };
        let err = handle_command_response(resp, "ok").unwrap_err();
        assert!(err.to_string().contains("Unexpected response type"));
    }

    #[test]
    fn generate_msg_id_returns_nonzero_most_of_the_time() {
        let ids: Vec<u64> = (0..10).map(|_| generate_msg_id()).collect();
        assert!(ids.iter().any(|&id| id != 0));
    }

    // ── mock gRPC servers ───────────────────────────────────────────────────

    struct MockChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for MockChannelManagerSvc {
        async fn command(
            &self,
            request: tonic::Request<ControlRequest>,
        ) -> Result<tonic::Response<ControlResponse>, tonic::Status> {
            let req = request.into_inner();
            let resp_payload = match req.payload {
                Some(RequestPayload::CreateChannel(_))
                | Some(RequestPayload::DeleteChannel(_))
                | Some(RequestPayload::AddParticipant(_))
                | Some(RequestPayload::DeleteParticipant(_)) => {
                    RespPayload::CommandResponse(CommandResponse {
                        msg_id: req.msg_id,
                        success: true,
                        error_msg: None,
                    })
                }
                Some(RequestPayload::ListChannels(_)) => RespPayload::ChannelsList(ChannelsList {
                    msg_id: req.msg_id,
                    channel_name: vec!["org/ns/chan1".to_string(), "org/ns/chan2".to_string()],
                }),
                Some(RequestPayload::ListParticipants(_)) => {
                    RespPayload::ParticipantsList(ParticipantsList {
                        msg_id: req.msg_id,
                        participant_name: vec!["org/ns/app1".to_string()],
                    })
                }
                None => {
                    return Err(tonic::Status::invalid_argument("missing payload"));
                }
            };
            Ok(tonic::Response::new(ControlResponse {
                msg_id: req.msg_id,
                payload: Some(resp_payload),
            }))
        }
    }

    struct ErrorChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for ErrorChannelManagerSvc {
        async fn command(
            &self,
            _request: tonic::Request<ControlRequest>,
        ) -> Result<tonic::Response<ControlResponse>, tonic::Status> {
            Err(tonic::Status::internal("forced server error"))
        }
    }

    struct NackChannelManagerSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for NackChannelManagerSvc {
        async fn command(
            &self,
            request: tonic::Request<ControlRequest>,
        ) -> Result<tonic::Response<ControlResponse>, tonic::Status> {
            let req = request.into_inner();
            let err_msg = match req.payload {
                Some(RequestPayload::ListChannels(_))
                | Some(RequestPayload::ListParticipants(_)) => "list denied",
                _ => "nack",
            };
            Ok(tonic::Response::new(ControlResponse {
                msg_id: req.msg_id,
                payload: Some(RespPayload::CommandResponse(CommandResponse {
                    msg_id: req.msg_id,
                    success: false,
                    error_msg: Some(err_msg.to_string()),
                })),
            }))
        }
    }

    struct UnexpectedPayloadSvc;

    #[tonic::async_trait]
    impl ChannelManagerService for UnexpectedPayloadSvc {
        async fn command(
            &self,
            request: tonic::Request<ControlRequest>,
        ) -> Result<tonic::Response<ControlResponse>, tonic::Status> {
            let req = request.into_inner();
            Ok(tonic::Response::new(ControlResponse {
                msg_id: req.msg_id,
                payload: Some(RespPayload::ChannelsList(ChannelsList {
                    msg_id: req.msg_id,
                    channel_name: vec![],
                })),
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

    // ── unexpected-payload tests ────────────────────────────────────────────

    #[tokio::test]
    async fn create_channel_unexpected_payload_fails() {
        let addr = spawn_svc(UnexpectedPayloadSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::CreateChannel {
                channel: "c".to_string(),
                disable_mls: false,
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("Unexpected response type"));
    }

    #[tokio::test]
    async fn delete_channel_unexpected_payload_fails() {
        let addr = spawn_svc(UnexpectedPayloadSvc).await;
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "c".to_string(),
            },
        };
        let err = run(&args, &make_opts(&addr)).await.unwrap_err();
        assert!(err.to_string().contains("Unexpected response type"));
    }
}
