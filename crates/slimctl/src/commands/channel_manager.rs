// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::io::Read;
use std::path::{Path, PathBuf};

use agntcy_slim_channel_manager::grant::{
    ChannelCreation, ChannelDeletion, Grant, GrantAction, SigningKey,
};
use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Args, Subcommand};

use crate::client::get_channel_manager_client;
use crate::proto::channel_manager::proto::v1::{
    AddParticipantRequest, ChannelInfo, CommandResponse, CreateChannelRequest,
    DeleteChannelRequest, DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
};
use crate::rpc;
use slim_config::grpc::client::ClientConfig;

/// Manage channels on a Channel Manager. slimctl never changes a channel on
/// its own authority: each change is signed by a channel's owner or an
/// operator, here with their key, or elsewhere and forwarded.
#[derive(Args)]
pub struct ChannelManagerArgs {
    #[command(subcommand)]
    pub command: ChannelManagerCommand,
}

/// How a create or delete request is signed.
#[derive(Args, Debug)]
pub struct RequestSignature {
    /// Ed25519 private key (PKCS#8 PEM) to sign the request with: the
    /// channel owner's, or an operator's
    #[arg(long, value_name = "PATH", group = "signature", requires = "audience")]
    pub key: Option<PathBuf>,
    /// The Channel Manager the request is for: its `audience`, which is its
    /// local-name unless configured otherwise
    #[arg(long, value_name = "AUDIENCE", env = "SLIMCTL_CM_AUDIENCE")]
    pub audience: Option<String>,
    /// File holding a request signed elsewhere ("-" reads stdin)
    #[arg(long, value_name = "PATH", group = "signature")]
    pub signed_request_file: Option<PathBuf>,
}

/// How a participant change is authorized: a grant signed with the owner's
/// key here, or one signed elsewhere.
#[derive(Args, Debug)]
pub struct GrantSignature {
    /// The channel owner's Ed25519 private key (PKCS#8 PEM) to sign a grant
    /// with
    #[arg(long, value_name = "PATH", group = "signature")]
    pub key: Option<PathBuf>,
    /// File holding a grant signed elsewhere ("-" reads stdin)
    #[arg(long, value_name = "PATH", group = "signature")]
    pub grant_file: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum ChannelManagerCommand {
    /// Create a channel, owned by whoever signs the request
    #[command(
        name = "create-channel",
        visible_alias = "cc",
        group(ArgGroup::new("signature").required(true))
    )]
    CreateChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Disable MLS for this channel (MLS is enabled by default)
        #[arg(long = "disable-mls", default_value_t = false)]
        disable_mls: bool,
        /// SLIM name (org/namespace/app) at which the owner can be asked to
        /// approve participant changes others request without a grant
        #[arg(long = "owner-callback-name", value_name = "NAME")]
        owner_callback_name: Option<String>,
        /// Delete the channel automatically this many seconds after creation
        #[arg(long = "ttl-seconds", value_name = "SECONDS")]
        ttl_seconds: Option<u64>,
        #[command(flatten)]
        signature: RequestSignature,
    },

    /// Delete a channel, with a request signed by its owner or an operator
    #[command(
        name = "delete-channel",
        visible_alias = "dc",
        group(ArgGroup::new("signature").required(true))
    )]
    DeleteChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        #[command(flatten)]
        signature: RequestSignature,
    },

    /// Add a participant to a channel, with a grant from its owner
    #[command(
        name = "add-participant",
        visible_alias = "ap",
        group(ArgGroup::new("signature").required(true))
    )]
    AddParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
        #[command(flatten)]
        signature: GrantSignature,
    },

    /// Remove a participant from a channel, with a grant from its owner
    #[command(
        name = "delete-participant",
        visible_alias = "dp",
        group(ArgGroup::new("signature").required(true))
    )]
    DeleteParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
        #[command(flatten)]
        signature: GrantSignature,
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

    /// Print the did:key of an Ed25519 private key, for example to list it
    /// in the Channel Manager's operators
    #[command(name = "did-key")]
    DidKey {
        /// Ed25519 private key (PKCS#8 PEM)
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
    },
}

/// How long a request or grant signed here stays valid: it's sent right
/// away, so minutes are plenty.
const SIGNED_HERE_LIFETIME_SECS: u64 = 5 * 60;

fn not_after() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + SIGNED_HERE_LIFETIME_SECS
}

fn load_key(path: &Path) -> Result<SigningKey> {
    let pem = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read key file: {}", path.display()))?;
    SigningKey::from_pkcs8_pem(&pem)
        .with_context(|| format!("not an Ed25519 PKCS#8 PEM key: {}", path.display()))
}

/// Reads a signed request or grant from `path` ("-" for stdin).
fn read_signed(path: &Path) -> Result<Vec<u8>> {
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

/// The key and audience to sign with, if signing here.
fn signing_key(signature: &RequestSignature) -> Result<Option<(SigningKey, String)>> {
    let Some(path) = &signature.key else {
        return Ok(None);
    };
    let audience = signature
        .audience
        .clone()
        .context("--audience is needed to sign with --key")?;
    Ok(Some((load_key(path)?, audience)))
}

/// The create request's `owner_proof`, and the owner it makes when signed
/// here.
fn create_proof(
    signature: &RequestSignature,
    request: &CreateChannelRequest,
) -> Result<(Vec<u8>, Option<String>)> {
    match (signing_key(signature)?, &signature.signed_request_file) {
        (Some((key, audience)), _) => {
            let creation = ChannelCreation {
                audience,
                channel: request.channel_name.clone(),
                owner: key.did_key().to_string(),
                mls_enabled: request.mls_enabled,
                ttl_seconds: request.ttl_seconds,
                owner_callback_name: request.owner_callback_name.clone(),
                not_after: not_after(),
                nonce: uuid::Uuid::new_v4().to_string(),
            };
            let proof = key.sign_create(&creation)?;
            Ok((proof, Some(creation.owner)))
        }
        (None, Some(path)) => Ok((read_signed(path)?, None)),
        (None, None) => bail!("--key or --signed-request-file is required"),
    }
}

fn delete_proof(signature: &RequestSignature, channel: &str) -> Result<Vec<u8>> {
    match (signing_key(signature)?, &signature.signed_request_file) {
        (Some((key, audience)), _) => Ok(key.sign_delete(&ChannelDeletion {
            audience,
            channel: channel.to_string(),
            not_after: not_after(),
            nonce: uuid::Uuid::new_v4().to_string(),
        })),
        (None, Some(path)) => read_signed(path),
        (None, None) => bail!("--key or --signed-request-file is required"),
    }
}

fn grant(
    signature: &GrantSignature,
    channel: &str,
    participant: &str,
    action: GrantAction,
) -> Result<Vec<u8>> {
    match (&signature.key, &signature.grant_file) {
        (Some(path), _) => Ok(load_key(path)?.sign_grant(&Grant {
            channel: channel.to_string(),
            invitee: participant.to_string(),
            action,
            role: "member".to_string(),
            not_after: not_after(),
            nonce: uuid::Uuid::new_v4().to_string(),
        })),
        (None, Some(path)) => read_signed(path),
        (None, None) => bail!("--key or --grant-file is required"),
    }
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
    // Needs no Channel Manager.
    if let ChannelManagerCommand::DidKey { key } = &args.command {
        println!("{}", load_key(key)?.did_key());
        return Ok(());
    }

    let mut client = get_channel_manager_client(opts).await?;

    match &args.command {
        ChannelManagerCommand::CreateChannel {
            channel,
            disable_mls,
            owner_callback_name,
            ttl_seconds,
            signature,
        } => {
            let mut request = CreateChannelRequest {
                channel_name: channel.clone(),
                mls_enabled: !disable_mls,
                owner_callback_name: owner_callback_name.clone(),
                ttl_seconds: *ttl_seconds,
                owner_proof: None,
            };
            let (proof, owner) = create_proof(signature, &request)?;
            request.owner_proof = Some(proof);
            let response = rpc!(client, create_channel, request);
            let created = match owner {
                Some(owner) => format!("Channel {channel} created, owned by {owner}"),
                None => format!("Channel {channel} created"),
            };
            check_command_response(response, &created)
        }

        ChannelManagerCommand::DeleteChannel { channel, signature } => {
            let request = DeleteChannelRequest {
                channel_name: channel.clone(),
                owner_proof: Some(delete_proof(signature, channel)?),
            };
            let response = rpc!(client, delete_channel, request);
            check_command_response(response, &format!("Channel {channel} deleted"))
        }

        ChannelManagerCommand::AddParticipant {
            channel,
            participant,
            signature,
        } => {
            let request = AddParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
                grant: Some(grant(signature, channel, participant, GrantAction::Add)?),
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
            signature,
        } => {
            let request = DeleteParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
                grant: Some(grant(signature, channel, participant, GrantAction::Delete)?),
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

        ChannelManagerCommand::DidKey { .. } => unreachable!("handled before connecting"),
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
                signature: forwarded(file.path()),
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
                signature: forwarded(file.path()),
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
                signature: forwarded(file.path()),
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
        assert_eq!(read_signed(file.path()).unwrap(), SIGNED.to_vec());
    }

    #[test]
    fn a_missing_signed_request_file_is_an_error() {
        let err = read_signed(Path::new("/nonexistent/delete.json")).unwrap_err();
        assert!(
            err.to_string()
                .contains("failed to read signed request file")
        );
    }

    // ── signing ─────────────────────────────────────────────────────────────

    use agntcy_slim_channel_manager::grant::{DidKeyEd25519Verifier, GrantVerifier};
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::Ed25519KeyPair;

    const AUDIENCE: &str = "org/ns/channel-manager";

    /// A request signed elsewhere, forwarded from `path`.
    fn forwarded(path: &Path) -> RequestSignature {
        RequestSignature {
            key: None,
            audience: None,
            signed_request_file: Some(path.to_path_buf()),
        }
    }

    /// A fresh Ed25519 key in a PKCS#8 PEM file, and its did:key.
    fn key_file() -> (tempfile::NamedTempFile, String) {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let body =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pkcs8.as_ref());
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &pem).unwrap();
        let did = SigningKey::from_pkcs8_pem(&pem)
            .unwrap()
            .did_key()
            .to_string();
        (file, did)
    }

    fn signed_with(key: &tempfile::NamedTempFile) -> RequestSignature {
        RequestSignature {
            key: Some(key.path().to_path_buf()),
            audience: Some(AUDIENCE.to_string()),
            signed_request_file: None,
        }
    }

    /// Records the signed bytes each mutating request carries, by RPC.
    #[derive(Default)]
    struct RecordingSvc {
        calls: std::sync::Arc<tokio::sync::Mutex<Vec<(&'static str, Option<Vec<u8>>)>>>,
    }

    impl RecordingSvc {
        async fn record(&self, rpc: &'static str, signed: Option<Vec<u8>>) -> CommandResponse {
            self.calls.lock().await.push((rpc, signed));
            CommandResponse {
                success: true,
                error_msg: None,
            }
        }
    }

    #[tonic::async_trait]
    impl ChannelManagerService for RecordingSvc {
        async fn create_channel(
            &self,
            request: tonic::Request<CreateChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            let signed = request.into_inner().owner_proof;
            Ok(tonic::Response::new(self.record("create", signed).await))
        }

        async fn delete_channel(
            &self,
            request: tonic::Request<DeleteChannelRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            let signed = request.into_inner().owner_proof;
            Ok(tonic::Response::new(self.record("delete", signed).await))
        }

        async fn add_participant(
            &self,
            request: tonic::Request<AddParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            let signed = request.into_inner().grant;
            Ok(tonic::Response::new(self.record("add", signed).await))
        }

        async fn delete_participant(
            &self,
            request: tonic::Request<DeleteParticipantRequest>,
        ) -> Result<tonic::Response<CommandResponse>, tonic::Status> {
            let signed = request.into_inner().grant;
            Ok(tonic::Response::new(self.record("remove", signed).await))
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

    /// Runs `command` against a [`RecordingSvc`], returning the one signed
    /// payload it sent.
    async fn send(command: ChannelManagerCommand) -> (&'static str, Vec<u8>) {
        let svc = RecordingSvc::default();
        let calls = svc.calls.clone();
        let addr = spawn_svc(svc).await;
        run(&ChannelManagerArgs { command }, &make_opts(&addr))
            .await
            .unwrap();
        let mut calls = calls.lock().await;
        assert_eq!(calls.len(), 1);
        let (rpc, signed) = calls.pop().unwrap();
        (rpc, signed.expect("no signed payload sent"))
    }

    #[tokio::test]
    async fn create_channel_signs_with_the_key_its_holder_owns() {
        let (key, did) = key_file();
        let (rpc, proof) = send(ChannelManagerCommand::CreateChannel {
            channel: "org/ns/chan".to_string(),
            disable_mls: false,
            owner_callback_name: Some("org/ns/admin-endpoint".to_string()),
            ttl_seconds: Some(3600),
            signature: signed_with(&key),
        })
        .await;
        assert_eq!(rpc, "create");
        let creation = DidKeyEd25519Verifier.verify_create(&proof).unwrap();
        assert_eq!(creation.owner, did);
        assert_eq!(creation.audience, AUDIENCE);
        assert_eq!(creation.channel, "org/ns/chan");
        assert!(creation.mls_enabled);
        assert_eq!(creation.ttl_seconds, Some(3600));
        assert_eq!(
            creation.owner_callback_name.as_deref(),
            Some("org/ns/admin-endpoint")
        );
    }

    #[tokio::test]
    async fn delete_channel_signs_with_the_key() {
        let (key, did) = key_file();
        let (rpc, proof) = send(ChannelManagerCommand::DeleteChannel {
            channel: "org/ns/chan".to_string(),
            signature: signed_with(&key),
        })
        .await;
        assert_eq!(rpc, "delete");
        let deletion = DidKeyEd25519Verifier.verify_delete(&did, &proof).unwrap();
        assert_eq!(
            (deletion.channel.as_str(), deletion.audience.as_str()),
            ("org/ns/chan", AUDIENCE)
        );
    }

    #[tokio::test]
    async fn participant_changes_sign_a_grant_with_the_key() {
        let (key, did) = key_file();
        for (command, expected_rpc, action) in [
            (
                ChannelManagerCommand::AddParticipant {
                    channel: "org/ns/chan".to_string(),
                    participant: "org/ns/client".to_string(),
                    signature: GrantSignature {
                        key: Some(key.path().to_path_buf()),
                        grant_file: None,
                    },
                },
                "add",
                GrantAction::Add,
            ),
            (
                ChannelManagerCommand::DeleteParticipant {
                    channel: "org/ns/chan".to_string(),
                    participant: "org/ns/client".to_string(),
                    signature: GrantSignature {
                        key: Some(key.path().to_path_buf()),
                        grant_file: None,
                    },
                },
                "remove",
                GrantAction::Delete,
            ),
        ] {
            let (rpc, grant) = send(command).await;
            assert_eq!(rpc, expected_rpc);
            let grant = DidKeyEd25519Verifier.verify(&did, &grant).unwrap();
            assert_eq!(
                (grant.channel.as_str(), grant.invitee.as_str(), grant.action),
                ("org/ns/chan", "org/ns/client", action)
            );
        }
    }

    #[tokio::test]
    async fn requests_signed_elsewhere_are_forwarded_unread() {
        let file = signed_request_file();
        let (rpc, proof) = send(ChannelManagerCommand::DeleteChannel {
            channel: "org/ns/chan".to_string(),
            signature: forwarded(file.path()),
        })
        .await;
        assert_eq!((rpc, proof), ("delete", SIGNED.to_vec()));

        let (rpc, grant) = send(ChannelManagerCommand::AddParticipant {
            channel: "org/ns/chan".to_string(),
            participant: "org/ns/client".to_string(),
            signature: GrantSignature {
                key: None,
                grant_file: Some(file.path().to_path_buf()),
            },
        })
        .await;
        assert_eq!((rpc, grant), ("add", SIGNED.to_vec()));
    }

    #[test]
    fn signing_with_a_key_needs_the_audience() {
        let (key, _) = key_file();
        let no_audience = RequestSignature {
            audience: None,
            ..signed_with(&key)
        };
        let err = delete_proof(&no_audience, "org/ns/chan").unwrap_err();
        assert!(err.to_string().contains("--audience"), "{err}");
    }

    #[test]
    fn a_key_file_that_is_not_an_ed25519_key_is_refused() {
        let file = signed_request_file();
        let err = load_key(file.path()).err().unwrap();
        assert!(err.to_string().contains("not an Ed25519"), "{err}");
    }
}
