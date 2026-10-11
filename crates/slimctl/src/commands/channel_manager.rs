// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

use crate::client::get_channel_manager_client;
use crate::proto::channel_manager::proto::v1::{
    ChannelInfo, CommandResponse, DeleteChannelRequest, ListChannelsRequest,
    ListParticipantsRequest,
};
use crate::rpc;
use slim_config::grpc::client::ClientConfig;

/// Operations on a Channel Manager. slimctl never creates channels or changes
/// their participants on its own authority: applications do that with their
/// owner's signature. It inspects channels, and deletes one with a request
/// its owner or an operator signed.
#[derive(Args)]
pub struct ChannelManagerArgs {
    #[command(subcommand)]
    pub command: ChannelManagerCommand,
}

#[derive(Subcommand)]
pub enum ChannelManagerCommand {
    /// Delete a channel, with a delete request its owner or an operator signed
    #[command(name = "delete-channel", visible_alias = "dc")]
    DeleteChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// File holding the signed delete request ("-" reads stdin)
        #[arg(long = "signed-request-file", value_name = "PATH")]
        signed_request_file: PathBuf,
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

/// Reads a signed request from `path` ("-" for stdin).
fn read_signed_request(path: &Path) -> Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut signed = Vec::new();
        std::io::stdin()
            .read_to_end(&mut signed)
            .context("failed to read the signed request from stdin")?;
        return Ok(signed);
    }
    std::fs::read(path)
        .with_context(|| format!("failed to read signed request file: {}", path.display()))
}

/// One line of `list-channels` output: the name, then owner and expiry when
/// the channel has them.
fn describe_channel(info: &ChannelInfo) -> String {
    let mut details = Vec::new();
    if let Some(owner) = &info.owner {
        details.push(format!("owner: {owner}"));
    }
    if let Some(expires_at) = info.expires_at {
        details.push(format!("expires: {}", format_unix_time(expires_at)));
    }
    if details.is_empty() {
        info.channel_name.clone()
    } else {
        format!("{} ({})", info.channel_name, details.join(", "))
    }
}

fn format_unix_time(secs: u64) -> String {
    i64::try_from(secs)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| secs.to_string())
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
        ChannelManagerCommand::DeleteChannel {
            channel,
            signed_request_file,
        } => {
            let request = DeleteChannelRequest {
                channel_name: channel.clone(),
                owner_proof: Some(read_signed_request(signed_request_file)?),
            };
            let response = rpc!(client, delete_channel, request);
            check_command_response(response, &format!("Channel {channel} deleted successfully"))
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
            // A server predating ChannelInfo fills only the plain name list.
            let lines: Vec<String> = if response.channels.is_empty() {
                response.channel_name.clone()
            } else {
                response.channels.iter().map(describe_channel).collect()
            };
            println!("Channels ({}):", lines.len());
            for line in &lines {
                println!("  - {line}");
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

    /// Stands in for a signed delete request: slimctl forwards it unread.
    const SIGNED: &[u8] = b"{\"signed\":\"delete\"}";

    fn signed_request_file() -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), SIGNED).unwrap();
        file
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
                channels: vec![],
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
                channels: vec![],
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
    async fn delete_channel_succeeds() {
        let addr = spawn_svc(MockChannelManagerSvc).await;
        let file = signed_request_file();
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "org/ns/chan".to_string(),
                signed_request_file: file.path().to_path_buf(),
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
    async fn delete_channel_grpc_error_propagates() {
        let addr = spawn_svc(ErrorChannelManagerSvc).await;
        let file = signed_request_file();
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "c".to_string(),
                signed_request_file: file.path().to_path_buf(),
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
    async fn delete_channel_nack_fails() {
        let addr = spawn_svc(NackChannelManagerSvc).await;
        let file = signed_request_file();
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "c".to_string(),
                signed_request_file: file.path().to_path_buf(),
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

    #[test]
    fn describes_a_channel_with_owner_and_expiry() {
        let info = ChannelInfo {
            channel_name: "org/ns/chan".to_string(),
            owner: Some("did:key:z6Mkowner".to_string()),
            expires_at: Some(1_800_000_000),
        };
        assert_eq!(
            describe_channel(&info),
            "org/ns/chan (owner: did:key:z6Mkowner, expires: 2027-01-15T08:00:00Z)"
        );
    }

    #[test]
    fn describes_a_channel_without_owner_or_expiry_by_name_alone() {
        let info = ChannelInfo {
            channel_name: "org/ns/chan".to_string(),
            owner: None,
            expires_at: None,
        };
        assert_eq!(describe_channel(&info), "org/ns/chan");
    }

    #[test]
    fn reads_a_signed_request_file() {
        let file = signed_request_file();
        assert_eq!(read_signed_request(file.path()).unwrap(), SIGNED.to_vec());
    }

    #[test]
    fn a_missing_signed_request_file_is_an_error() {
        let err = read_signed_request(Path::new("/nonexistent/delete.json")).unwrap_err();
        assert!(
            err.to_string()
                .contains("failed to read signed request file")
        );
    }

    /// Records the owner_proof of each delete request it receives.
    #[derive(Default)]
    struct RecordingSvc {
        proofs: std::sync::Arc<tokio::sync::Mutex<Vec<Option<Vec<u8>>>>>,
    }

    #[tonic::async_trait]
    impl ChannelManagerService for RecordingSvc {
        async fn create_channel(
            &self,
            _request: tonic::Request<CreateChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }

        async fn delete_channel(
            &self,
            request: tonic::Request<DeleteChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            self.proofs
                .lock()
                .await
                .push(request.into_inner().owner_proof);
            Ok(tonic::Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn add_participant(
            &self,
            _request: tonic::Request<AddParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }

        async fn delete_participant(
            &self,
            _request: tonic::Request<DeleteParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }

        async fn list_channels(
            &self,
            _request: tonic::Request<ListChannelsRequest>,
        ) -> Result<tonic::Response<ListChannelsResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }

        async fn list_participants(
            &self,
            _request: tonic::Request<ListParticipantsRequest>,
        ) -> Result<tonic::Response<ListParticipantsResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }
    }

    #[tokio::test]
    async fn delete_channel_forwards_the_signed_request() {
        let svc = RecordingSvc::default();
        let proofs = svc.proofs.clone();
        let addr = spawn_svc(svc).await;
        let file = signed_request_file();
        let args = ChannelManagerArgs {
            command: ChannelManagerCommand::DeleteChannel {
                channel: "org/ns/chan".to_string(),
                signed_request_file: file.path().to_path_buf(),
            },
        };
        run(&args, &make_opts(&addr)).await.unwrap();
        assert_eq!(*proofs.lock().await, vec![Some(SIGNED.to_vec())]);
    }
}
